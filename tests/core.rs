use sifs::{
    Chunk, HashingEncoder, SearchMode, SearchOptions, SearchResult, SifsIndex, format_results,
};
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
            .search_with(
                "authenticate token",
                &SearchOptions::new(3).with_mode(SearchMode::Bm25)
            )
            .is_empty()
    );
    assert!(
        !index
            .search_with(
                "authentication",
                &SearchOptions::new(3).with_mode(SearchMode::Semantic)
            )
            .is_empty()
    );
    assert!(
        !index
            .search_with(
                "UserService",
                &SearchOptions::new(3).with_mode(SearchMode::Hybrid)
            )
            .is_empty()
    );
    assert!(
        index
            .search("   ", 3, SearchMode::Hybrid, None, None, None)
            .is_empty()
    );

    let paths = vec!["utils.py".to_owned()];
    let filtered = index.search_with("format", &SearchOptions::new(3).with_paths(paths));
    assert!(filtered.iter().all(|r| r.chunk.file_path == "utils.py"));
}

#[test]
fn formatted_results_include_source_mode() {
    let results = vec![SearchResult {
        chunk: chunk("def authenticate(): pass", "auth.py"),
        score: 0.75,
        source: SearchMode::Hybrid,
    }];

    let output = format_results("Search results", &results);

    assert!(output.contains("## 1. auth.py:1-1  [score=0.750, source=hybrid]"));
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
