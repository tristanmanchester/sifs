use ndarray::Array2;
use sifs::{
    CacheConfig, Chunk, Encoder, EncoderSpec, HashingEncoder, IndexOptions, ModelLoadPolicy,
    ModelOptions, SearchMode, SearchOptions, SearchResult, SifsIndex, format_results,
};
use std::fs;
use std::process::Command;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn chunk(content: &str, file_path: &str) -> Chunk {
    Chunk {
        content: content.to_owned(),
        file_path: file_path.to_owned(),
        start_line: 1,
        end_line: content.lines().count().max(1),
        language: Some("python".to_owned()),
        symbols: Vec::new(),
        breadcrumbs: Vec::new(),
    }
}

struct CountingEncoder {
    calls: Arc<AtomicUsize>,
}

impl Encoder for CountingEncoder {
    fn encode(&self, texts: &[String]) -> Array2<f32> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut values = Array2::zeros((texts.len(), 2));
        for (idx, text) in texts.iter().enumerate() {
            values[[idx, 0]] = if text.contains("auth") { 1.0 } else { 0.0 };
            values[[idx, 1]] = 1.0;
        }
        values
    }

    fn dim(&self) -> usize {
        2
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
fn search_options_can_bypass_query_cache() {
    let calls = Arc::new(AtomicUsize::new(0));
    let chunks = vec![
        chunk("fn authenticate_token() {}", "src/auth.rs"),
        chunk("fn render_chart() {}", "src/chart.rs"),
    ];
    let index = SifsIndex::from_chunks(
        Box::new(CountingEncoder {
            calls: calls.clone(),
        }),
        chunks,
    )
    .unwrap();
    let cached = SearchOptions::new(1)
        .with_mode(SearchMode::Semantic)
        .with_cache(true);
    let uncached = SearchOptions::new(1)
        .with_mode(SearchMode::Semantic)
        .with_cache(false);

    index.search_with("auth token", &cached).unwrap();
    index.search_with("auth token", &cached).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    index.search_with("auth token", &uncached).unwrap();
    index.search_with("auth token", &uncached).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}

#[test]
fn formatted_results_include_source_mode() {
    let results = vec![SearchResult {
        chunk: chunk("def authenticate(): pass", "auth.py"),
        score: 0.75,
        source: SearchMode::Hybrid,
        explanation: None,
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
fn indexing_skips_non_utf8_files_with_warning() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("valid.rs"), "fn valid_symbol() {}\n").unwrap();
    fs::write(dir.path().join("invalid.rs"), [0xff, 0xfe, 0xfd]).unwrap();

    let index = SifsIndex::from_path_sparse(dir.path()).unwrap();

    assert_eq!(index.stats().indexed_files, 1);
    assert!(index.indexed_files().contains(&"valid.rs".to_owned()));
    assert!(index.warnings().iter().any(|warning| {
        warning.path == "invalid.rs"
            && warning
                .message
                .contains("stream did not contain valid UTF-8")
    }));
}

#[test]
fn sparse_cache_preserves_index_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("valid.rs"), "fn valid_symbol() {}\n").unwrap();
    fs::write(dir.path().join("invalid.rs"), [0xff, 0xfe, 0xfd]).unwrap();
    let options = || IndexOptions::sparse().with_cache(CacheConfig::Custom(cache.path().into()));

    let first = SifsIndex::from_path_with_index_options(dir.path(), options()).unwrap();
    assert!(first.warnings().iter().any(|warning| {
        warning.path == "invalid.rs"
            && warning
                .message
                .contains("stream did not contain valid UTF-8")
    }));

    let cached = SifsIndex::from_path_with_index_options(dir.path(), options()).unwrap();
    assert!(cached.warnings().iter().any(|warning| {
        warning.path == "invalid.rs"
            && warning
                .message
                .contains("stream did not contain valid UTF-8")
    }));
}

#[test]
fn bm25_path_search_is_model_free_with_no_download() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate_token(token):\n    return token == 'secret'\n",
    )
    .unwrap();
    let index = SifsIndex::from_path_sparse(dir.path()).unwrap();

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
fn sparse_only_index_reports_semantic_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    pass\n",
    )
    .unwrap();
    let index = SifsIndex::from_path_sparse(dir.path()).unwrap();

    let err = index
        .search_with(
            "authentication",
            &SearchOptions::new(3).with_mode(SearchMode::Semantic),
        )
        .unwrap_err();

    assert!(err.to_string().contains("sparse-only index"));
    assert!(err.to_string().contains("SearchMode::Bm25"));
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
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    return True\n",
    )
    .unwrap();
    let options = ModelOptions::new(
        Some("__force_hashing_fallback__"),
        ModelLoadPolicy::NoDownload,
    );

    let index = SifsIndex::from_path_with_index_options(
        dir.path(),
        IndexOptions::new(options.clone()).with_cache(CacheConfig::Project),
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
            .any(|name| name.starts_with("semantic-v5-"))
    );

    let index = SifsIndex::from_path_with_index_options(
        dir.path(),
        IndexOptions::new(options).with_cache(CacheConfig::Project),
    )
    .unwrap();
    assert_eq!(index.is_fresh(), Some(true));
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    return False\n",
    )
    .unwrap();
    assert_eq!(index.is_fresh(), Some(false));
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
fn hashing_encoder_spec_supports_semantic_without_model_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    return True\n",
    )
    .unwrap();

    let index = SifsIndex::from_path_with_encoder_spec(
        dir.path(),
        EncoderSpec::hashing(),
        None,
        None,
        false,
    )
    .unwrap();
    let results = index
        .search_with(
            "authentication",
            &SearchOptions::new(3).with_mode(SearchMode::Semantic),
        )
        .unwrap();

    assert!(!results.is_empty());
    assert!(index.semantic_loaded());
}

#[test]
fn cli_bm25_offline_succeeds_without_model() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate_token(token):\n    return token == 'secret'\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_sifs"))
        .args([
            "search",
            "authenticate_token",
            "--source",
            dir.path().to_str().unwrap(),
            "--mode",
            "bm25",
            "--offline",
            "--model",
            "__missing_cli_bm25_model__",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("main.py"));
}

#[test]
fn cli_semantic_offline_missing_model_fails_helpfully() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    pass\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_sifs"))
        .args([
            "search",
            "authentication",
            "--source",
            dir.path().to_str().unwrap(),
            "--mode",
            "semantic",
            "--offline",
            "--model",
            "__missing_cli_semantic_model__",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not available locally"));
}

#[test]
fn cli_hashing_encoder_supports_semantic_without_model() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def authenticate():\n    return True\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_sifs"))
        .args([
            "search",
            "authentication",
            "--source",
            dir.path().to_str().unwrap(),
            "--mode",
            "semantic",
            "--encoder",
            "hashing",
            "--offline",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("main.py"));
}

#[test]
fn cli_doctor_reports_hashing_readiness() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sifs"))
        .args([
            "doctor",
            "--source",
            dir.path().to_str().unwrap(),
            "--encoder",
            "hashing",
            "--offline",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("SIFS doctor"));
    assert!(stdout.contains("Semantic readiness: ready without model files"));
}
