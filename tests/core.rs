use sifs::{
    CacheConfig, Chunk, HashingEncoder, IndexOptions, ModelLoadPolicy, ModelOptions, SearchMode,
    SearchOptions, SearchResult, SifsIndex, cache_summary, format_results,
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

#[test]
fn bm25_path_search_is_model_free_with_no_download() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate_token(token):\n    return token == 'secret'\n",
    )
    .unwrap();
    let index = SifsIndex::from_path_with_model_options(
        dir.path(),
        ModelOptions::new(
            Some("__missing_model_for_bm25_test__"),
            ModelLoadPolicy::NoDownload,
        ),
        None,
        None,
        false,
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
    let index = SifsIndex::from_path_with_model_options(
        dir.path(),
        ModelOptions::new(
            Some("__missing_model_for_semantic_test__"),
            ModelLoadPolicy::NoDownload,
        ),
        None,
        None,
        false,
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
    let cache_dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    return True\n",
    )
    .unwrap();
    let options = ModelOptions::new(
        Some("__force_hashing_fallback__"),
        ModelLoadPolicy::NoDownload,
    );
    let index_options = IndexOptions::new(options.clone())
        .with_cache(CacheConfig::Custom(cache_dir.path().to_path_buf()));

    let index = SifsIndex::from_path_with_index_options(dir.path(), index_options.clone()).unwrap();
    assert!(!index.semantic_loaded());
    let first_results = index
        .search_with(
            "authentication",
            &SearchOptions::new(3).with_mode(SearchMode::Semantic),
        )
        .unwrap();
    assert!(!first_results.is_empty());
    assert!(index.semantic_loaded());

    let summary = cache_summary(cache_dir.path());
    assert!(summary.entries >= 1);
    assert!(summary.files >= 2);
    assert!(!dir.path().join(".sifs").exists());

    let index = SifsIndex::from_path_with_index_options(dir.path(), index_options).unwrap();
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
fn default_cache_does_not_write_project_local_state() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("main.py"), "def foo():\n    pass\n").unwrap();

    let index = SifsIndex::from_path_with_model_options(
        dir.path(),
        ModelOptions::new(
            Some("__missing_model_for_default_cache_test__"),
            ModelLoadPolicy::NoDownload,
        ),
        None,
        None,
        false,
    )
    .unwrap();

    assert!(!index.chunks.is_empty());
    assert!(!dir.path().join(".sifs").exists());
}

#[test]
fn project_cache_writes_project_local_state() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("main.py"), "def foo():\n    pass\n").unwrap();
    let options = IndexOptions::new(ModelOptions::new(
        Some("__missing_model_for_project_cache_test__"),
        ModelLoadPolicy::NoDownload,
    ))
    .with_cache(CacheConfig::Project);

    let index = SifsIndex::from_path_with_index_options(dir.path(), options).unwrap();

    assert!(!index.chunks.is_empty());
    assert!(
        dir.path()
            .join(".sifs")
            .join("index-v3-sparse.bin")
            .exists()
    );
}

#[test]
fn disabled_cache_writes_no_persistent_state() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("main.py"), "def foo():\n    pass\n").unwrap();
    let options = IndexOptions::new(ModelOptions::new(
        Some("__missing_model_for_disabled_cache_test__"),
        ModelLoadPolicy::NoDownload,
    ))
    .with_cache(CacheConfig::Disabled);

    let index = SifsIndex::from_path_with_index_options(dir.path(), options).unwrap();

    assert!(!index.chunks.is_empty());
    assert!(!dir.path().join(".sifs").exists());
}
