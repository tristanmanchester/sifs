use sifs::{
    CacheMode, Chunk, HashingEncoder, IndexOptions, ModelLoadPolicy, ModelOptions, SearchMode,
    SearchOptions, SearchResult, SifsIndex, format_results,
};
use std::fs;

fn chunk(content: &str, file_path: &str) -> Chunk {
    chunk_with_language(content, file_path, "python")
}

fn chunk_with_language(content: &str, file_path: &str, language: &str) -> Chunk {
    Chunk {
        content: content.to_owned(),
        file_path: file_path.to_owned(),
        start_line: 1,
        end_line: content.lines().count().max(1),
        language: Some(language.to_owned()),
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
            .unwrap()
            .is_empty()
    );
    assert!(
        !index
            .search_with(
                "authentication",
                &SearchOptions::new(3).with_mode(SearchMode::Semantic)
            )
            .unwrap()
            .is_empty()
    );
    assert!(
        !index
            .search_with(
                "UserService",
                &SearchOptions::new(3).with_mode(SearchMode::Hybrid)
            )
            .unwrap()
            .is_empty()
    );
    assert!(
        index
            .search("   ", 3, SearchMode::Hybrid, None, None, None)
            .unwrap()
            .is_empty()
    );

    let paths = vec!["utils.py".to_owned()];
    let filtered = index
        .search_with("format", &SearchOptions::new(3).with_paths(paths))
        .unwrap();
    assert!(filtered.iter().all(|r| r.chunk.file_path == "utils.py"));
}

#[test]
fn unknown_language_filter_returns_empty_results() {
    let chunks = vec![chunk_with_language(
        "fn auth_token() {}",
        "src/lib.rs",
        "rust",
    )];
    let index = SifsIndex::from_chunks(Box::new(HashingEncoder::new(256)), chunks).unwrap();

    let results = index
        .search_with(
            "auth_token",
            &SearchOptions::new(3)
                .with_mode(SearchMode::Bm25)
                .with_languages(["does-not-exist".to_owned()]),
        )
        .unwrap();

    assert!(results.is_empty());
}

#[test]
fn unknown_path_filter_returns_empty_results() {
    let chunks = vec![chunk_with_language(
        "fn auth_token() {}",
        "src/lib.rs",
        "rust",
    )];
    let index = SifsIndex::from_chunks(Box::new(HashingEncoder::new(256)), chunks).unwrap();

    let results = index
        .search_with(
            "auth_token",
            &SearchOptions::new(3)
                .with_mode(SearchMode::Bm25)
                .with_paths(["src/missing.rs".to_owned()]),
        )
        .unwrap();

    assert!(results.is_empty());
}

#[test]
fn language_and_path_filters_intersect() {
    let chunks = vec![
        chunk_with_language("fn auth_token() {}", "src/lib.rs", "rust"),
        chunk_with_language("def auth_token(): pass", "src/foo.py", "python"),
    ];
    let index = SifsIndex::from_chunks(Box::new(HashingEncoder::new(256)), chunks).unwrap();

    let mismatched = index
        .search_with(
            "auth_token",
            &SearchOptions::new(3)
                .with_mode(SearchMode::Bm25)
                .with_languages(["rust".to_owned()])
                .with_paths(["src/foo.py".to_owned()]),
        )
        .unwrap();
    let matched = index
        .search_with(
            "auth_token",
            &SearchOptions::new(3)
                .with_mode(SearchMode::Bm25)
                .with_languages(["rust".to_owned()])
                .with_paths(["src/lib.rs".to_owned()]),
        )
        .unwrap();

    assert!(mismatched.is_empty());
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].chunk.file_path, "src/lib.rs");
}

#[test]
fn multiple_values_inside_filter_group_are_ored() {
    let chunks = vec![
        chunk_with_language("fn auth_token() {}", "src/lib.rs", "rust"),
        chunk_with_language("def auth_token(): pass", "src/foo.py", "python"),
        chunk_with_language("function auth_token() {}", "src/foo.ts", "typescript"),
    ];
    let index = SifsIndex::from_chunks(Box::new(HashingEncoder::new(256)), chunks).unwrap();

    let results = index
        .search_with(
            "auth_token",
            &SearchOptions::new(10)
                .with_mode(SearchMode::Bm25)
                .with_languages(["rust".to_owned(), "python".to_owned()]),
        )
        .unwrap();

    assert_eq!(results.len(), 2);
    assert!(
        results
            .iter()
            .all(|result| result.chunk.language.as_deref() != Some("typescript"))
    );
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
    let index = SifsIndex::from_path_with_model_options_and_index_options(
        dir.path(),
        ModelOptions::new(
            Some("__force_hashing_fallback__"),
            ModelLoadPolicy::NoDownload,
        ),
        None,
        None,
        false,
        IndexOptions {
            cache_mode: CacheMode::Off,
        },
    )
    .unwrap();
    assert!(index.chunks.iter().all(|c| !c.file_path.ends_with(".md")));
    let index = SifsIndex::from_path_with_model_options_and_index_options(
        dir.path(),
        ModelOptions::new(
            Some("__force_hashing_fallback__"),
            ModelLoadPolicy::NoDownload,
        ),
        None,
        None,
        true,
        IndexOptions {
            cache_mode: CacheMode::Off,
        },
    )
    .unwrap();
    assert!(index.chunks.iter().any(|c| c.file_path.ends_with(".md")));
}

#[test]
fn bm25_path_search_is_model_free_with_no_download() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate_token(token):\n    return token == 'secret'\n",
    )
    .unwrap();
    let index = SifsIndex::from_path_with_model_options_and_index_options(
        dir.path(),
        ModelOptions::new(
            Some("__missing_model_for_bm25_test__"),
            ModelLoadPolicy::NoDownload,
        ),
        None,
        None,
        false,
        IndexOptions {
            cache_mode: CacheMode::Off,
        },
    )
    .unwrap();

    let results = index
        .search_with(
            "authenticate_token",
            &SearchOptions::new(3).with_mode(SearchMode::Bm25),
        )
        .unwrap();

    assert!(!results.is_empty());
    assert!(!index.semantic_loaded());
}

#[test]
fn semantic_search_reports_missing_model_with_no_download() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    pass\n",
    )
    .unwrap();
    let index = SifsIndex::from_path_with_model_options_and_index_options(
        dir.path(),
        ModelOptions::new(
            Some("__missing_model_for_semantic_test__"),
            ModelLoadPolicy::NoDownload,
        ),
        None,
        None,
        false,
        IndexOptions {
            cache_mode: CacheMode::Off,
        },
    )
    .unwrap();

    let err = index
        .search_with(
            "authentication",
            &SearchOptions::new(3).with_mode(SearchMode::Semantic),
        )
        .unwrap_err();

    assert!(err.to_string().contains("not available locally"));
}

#[test]
fn semantic_search_writes_and_reuses_dense_cache() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    return True\n",
    )
    .unwrap();
    let options = ModelOptions::new(
        Some("__force_hashing_fallback__"),
        ModelLoadPolicy::NoDownload,
    );

    let index = SifsIndex::from_path_with_model_options_and_index_options(
        dir.path(),
        options.clone(),
        None,
        None,
        false,
        IndexOptions {
            cache_mode: CacheMode::Local,
        },
    )
    .unwrap();
    assert!(!index.semantic_loaded());
    let first_results = index
        .search_with(
            "authentication",
            &SearchOptions::new(3).with_mode(SearchMode::Semantic),
        )
        .unwrap();
    assert!(!first_results.is_empty());
    assert!(index.semantic_loaded());

    let cache_files: Vec<_> = fs::read_dir(dir.path().join(".sifs"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        cache_files
            .iter()
            .any(|name| name.starts_with("semantic-v3-"))
    );

    let index = SifsIndex::from_path_with_model_options_and_index_options(
        dir.path(),
        options,
        None,
        None,
        false,
        IndexOptions {
            cache_mode: CacheMode::Local,
        },
    )
    .unwrap();
    assert!(!index.semantic_loaded());
    let second_results = index
        .search_with(
            "authentication",
            &SearchOptions::new(3).with_mode(SearchMode::Semantic),
        )
        .unwrap();

    assert_eq!(first_results.len(), second_results.len());
    assert_eq!(
        first_results[0].chunk.file_path,
        second_results[0].chunk.file_path
    );
    assert!(index.semantic_loaded());
}

#[test]
fn default_cache_does_not_create_repo_local_sifs_directory() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    pass\n",
    )
    .unwrap();

    let index = SifsIndex::from_path_with_model_options(
        dir.path(),
        ModelOptions::new(
            Some("__force_hashing_fallback__"),
            ModelLoadPolicy::NoDownload,
        ),
        None,
        None,
        false,
    )
    .unwrap();

    assert!(!index.chunks.is_empty());
    assert!(!dir.path().join(".sifs").exists());
    let _ = SifsIndex::clean_cache(dir.path(), CacheMode::Platform);
}

#[test]
fn local_cache_mode_creates_repo_local_sifs_directory_and_clean_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    pass\n",
    )
    .unwrap();

    let index = SifsIndex::from_path_with_model_options_and_index_options(
        dir.path(),
        ModelOptions::new(
            Some("__force_hashing_fallback__"),
            ModelLoadPolicy::NoDownload,
        ),
        None,
        None,
        false,
        IndexOptions {
            cache_mode: CacheMode::Local,
        },
    )
    .unwrap();

    assert!(!index.chunks.is_empty());
    assert!(dir.path().join(".sifs").exists());
    assert!(SifsIndex::clean_cache(dir.path(), CacheMode::Local).unwrap());
    assert!(!dir.path().join(".sifs").exists());
}

#[test]
fn cache_metadata_change_invalidates_local_cache() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    pass\n",
    )
    .unwrap();
    fs::write(dir.path().join("README.md"), "# Auth docs\n").unwrap();
    let options = ModelOptions::new(
        Some("__force_hashing_fallback__"),
        ModelLoadPolicy::NoDownload,
    );

    let code_only = SifsIndex::from_path_with_model_options_and_index_options(
        dir.path(),
        options.clone(),
        None,
        None,
        false,
        IndexOptions {
            cache_mode: CacheMode::Local,
        },
    )
    .unwrap();
    let with_docs = SifsIndex::from_path_with_model_options_and_index_options(
        dir.path(),
        options,
        None,
        None,
        true,
        IndexOptions {
            cache_mode: CacheMode::Local,
        },
    )
    .unwrap();

    assert!(
        code_only
            .chunks
            .iter()
            .all(|chunk| chunk.file_path != "README.md")
    );
    assert!(
        with_docs
            .chunks
            .iter()
            .any(|chunk| chunk.file_path == "README.md")
    );
}

#[test]
fn non_utf8_supported_file_is_skipped_with_warning() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    pass\n",
    )
    .unwrap();
    fs::write(dir.path().join("bad.py"), [0xff, 0xfe, 0xfd]).unwrap();

    let index = SifsIndex::from_path_with_model_options_and_index_options(
        dir.path(),
        ModelOptions::new(
            Some("__force_hashing_fallback__"),
            ModelLoadPolicy::NoDownload,
        ),
        None,
        None,
        false,
        IndexOptions {
            cache_mode: CacheMode::Off,
        },
    )
    .unwrap();

    assert!(index.chunks.iter().all(|chunk| chunk.file_path != "bad.py"));
    assert!(
        index
            .warnings()
            .iter()
            .any(|warning| warning.path == "bad.py")
    );
}

#[test]
fn oversized_supported_file_is_skipped_with_warning() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    pass\n",
    )
    .unwrap();
    fs::write(dir.path().join("large.py"), "x".repeat(5 * 1024 * 1024 + 1)).unwrap();

    let index = SifsIndex::from_path_with_model_options_and_index_options(
        dir.path(),
        ModelOptions::new(
            Some("__force_hashing_fallback__"),
            ModelLoadPolicy::NoDownload,
        ),
        None,
        None,
        false,
        IndexOptions {
            cache_mode: CacheMode::Off,
        },
    )
    .unwrap();

    assert!(
        index
            .chunks
            .iter()
            .all(|chunk| chunk.file_path != "large.py")
    );
    assert!(
        index
            .warnings()
            .iter()
            .any(|warning| warning.path == "large.py" && warning.message.contains("larger"))
    );
}

#[test]
fn indexing_fails_when_all_candidates_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("bad.py"), [0xff, 0xfe, 0xfd]).unwrap();

    let err = match SifsIndex::from_path_with_model_options_and_index_options(
        dir.path(),
        ModelOptions::new(
            Some("__force_hashing_fallback__"),
            ModelLoadPolicy::NoDownload,
        ),
        None,
        None,
        false,
        IndexOptions {
            cache_mode: CacheMode::Off,
        },
    ) {
        Ok(_) => panic!("indexing should fail when all candidates are skipped"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("No supported files found"));
}
