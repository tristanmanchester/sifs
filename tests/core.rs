use sifs::chunker::chunk_lines;
use sifs::index::SifsIndex;
use sifs::model2vec::HashingEncoder;
use sifs::ranking::{apply_query_boost, rerank_topk, resolve_alpha};
use sifs::tokens::{split_identifier, tokenize};
use sifs::types::{Chunk, SearchMode};
use std::collections::HashMap;
use std::fs;

fn chunk(content: &str, file_path: &str) -> Chunk {
    Chunk {
        content: content.to_owned(),
        file_path: file_path.to_owned(),
        start_line: 1,
        end_line: content.lines().count().max(1),
        language: Some("python".to_owned()),
    }
}

#[test]
fn tokenization_matches_identifier_expansion() {
    assert_eq!(
        split_identifier("HandlerStack"),
        vec!["handlerstack", "handler", "stack"]
    );
    assert_eq!(split_identifier("my_func"), vec!["my_func", "my", "func"]);
    assert_eq!(
        tokenize("getHTTPResponse my_func"),
        vec![
            "gethttpresponse",
            "get",
            "http",
            "response",
            "my_func",
            "my",
            "func"
        ]
    );
}

#[test]
fn chunk_lines_uses_overlap_and_locations() {
    let source = (1..=60).map(|i| format!("line {i}\n")).collect::<String>();
    let chunks = chunk_lines(&source, "src/foo.py", Some("python".to_owned()), 50, 5);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].start_line, 1);
    assert_eq!(chunks[0].end_line, 50);
    assert_eq!(chunks[1].start_line, 46);
}

#[test]
fn ranking_boosts_definitions_and_penalizes_tests() {
    let defining = chunk("class MyService:\n    pass", "src/my_service.py");
    let other = chunk("x = MyService()", "src/utils.py");
    let chunks = vec![defining, other];
    let mut scores = HashMap::new();
    scores.insert(1usize, 0.4);
    let boosted = apply_query_boost(&scores, "MyService", &chunks);
    assert!(boosted[&0] > boosted[&1]);

    let regular = chunk("def impl(): pass", "src/regular.py");
    let test = chunk("def impl(): pass", "tests/test_auth.py");
    let chunks = vec![regular, test];
    let scores = HashMap::from([(0usize, 1.0), (1usize, 1.0)]);
    let ranked = rerank_topk(&scores, &chunks, 2, true);
    assert_eq!(ranked[0].0, 0);
}

#[test]
fn alpha_detection_matches_symbol_vs_natural_language() {
    assert_eq!(resolve_alpha("MyService", None), 0.3);
    assert_eq!(resolve_alpha("how does routing work", None), 0.55);
    assert_eq!(
        resolve_alpha("request validation and error handling", None),
        0.3
    );
    assert_eq!(resolve_alpha("MyService", Some(0.7)), 0.7);
}

#[test]
fn index_search_modes_and_filters_work() {
    let chunks = vec![
        chunk(
            "def authenticate(token):\n    return token == 'secret'",
            "auth.py",
        ),
        chunk("def login(username, password):\n    pass", "auth.py"),
        chunk("class UserService:\n    pass", "users.py"),
        chunk("def format_date(dt):\n    return str(dt)", "utils.py"),
    ];
    let index = SifsIndex::from_chunks(Box::new(HashingEncoder::new(256)), chunks).unwrap();
    assert!(
        !index
            .search("authenticate token", 3, SearchMode::Bm25, None, None, None)
            .is_empty()
    );
    assert!(
        !index
            .search("authentication", 3, SearchMode::Semantic, None, None, None)
            .is_empty()
    );
    assert!(
        !index
            .search("UserService", 3, SearchMode::Hybrid, None, None, None)
            .is_empty()
    );
    assert!(
        index
            .search("   ", 3, SearchMode::Hybrid, None, None, None)
            .is_empty()
    );

    let paths = vec!["utils.py".to_owned()];
    let filtered = index.search("format", 3, SearchMode::Hybrid, None, None, Some(&paths));
    assert!(filtered.iter().all(|r| r.chunk.file_path == "utils.py"));
}

#[test]
fn from_path_respects_markdown_option() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("main.py"), "def foo():\n    pass\n").unwrap();
    fs::write(dir.path().join("README.md"), "# Docs\n").unwrap();
    let index = SifsIndex::from_path_with_options(
        dir.path(),
        Some("__force_hashing_fallback__"),
        None,
        None,
        false,
    )
    .unwrap();
    assert!(index.chunks.iter().all(|c| !c.file_path.ends_with(".md")));
    let index = SifsIndex::from_path_with_options(
        dir.path(),
        Some("__force_hashing_fallback__"),
        None,
        None,
        true,
    )
    .unwrap();
    assert!(index.chunks.iter().any(|c| c.file_path.ends_with(".md")));
}

#[test]
fn file_walker_honors_root_gitignore_negation() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("out")).unwrap();
    fs::create_dir_all(dir.path().join("ios")).unwrap();
    fs::create_dir_all(dir.path().join(".expo")).unwrap();
    fs::write(dir.path().join("out/a.py"), "x = 1\n").unwrap();
    fs::write(dir.path().join("out/keep.py"), "x = 1\n").unwrap();
    fs::write(dir.path().join("ios/Generated.swift"), "let x = 1\n").unwrap();
    fs::write(dir.path().join(".expo/state.ts"), "export const x = 1\n").unwrap();
    fs::write(
        dir.path().join(".gitignore"),
        "out/*\n!out/keep.py\n/ios\n.expo/\n",
    )
    .unwrap();
    let extensions = std::collections::HashSet::from([".py".to_owned()]);
    let found: std::collections::HashSet<String> =
        sifs::file_walker::walk_files(dir.path(), &extensions, None)
            .into_iter()
            .map(|p| {
                p.strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
    assert_eq!(
        found,
        std::collections::HashSet::from(["out/keep.py".to_owned()])
    );
}

#[test]
fn code_chunker_uses_tree_sitter_boundaries() {
    let source = "def alpha():\n    return 1\n\n".repeat(80);
    let chunks = sifs::chunker::chunk_source(&source, "many.py", Some("python".to_owned()));
    assert!(chunks.len() > 1);
    assert_eq!(chunks.first().unwrap().start_line, 1);
    assert!(chunks.windows(2).all(|w| w[0].end_line <= w[1].start_line));
    assert!(chunks.iter().all(|chunk| chunk.content.len() <= 1800));
}
