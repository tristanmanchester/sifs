use sifs::daemon::{
    DAEMON_PROTOCOL_VERSION, DaemonRequest, DaemonRequestEnvelope, IndexIdentity, IndexManager,
    IndexRuntimeOptions, SourceKind, SourceSpec,
};
use sifs::{CacheConfig, EncoderSpec, SearchMode, SearchOptions};

#[test]
fn source_spec_canonicalizes_local_paths() {
    let dir = tempfile::tempdir().unwrap();

    let source = SourceSpec::resolve(
        dir.path().to_string_lossy(),
        Some("ignored".to_owned()),
        false,
    )
    .unwrap();

    assert_eq!(source.kind, SourceKind::LocalPath);
    assert_eq!(
        source.source,
        dir.path().canonicalize().unwrap().to_string_lossy()
    );
    assert_eq!(source.ref_name, None);
    assert!(source.cache_key().starts_with("path:"));
}

#[test]
fn source_spec_preserves_git_ref_and_rejects_offline_git() {
    let source = SourceSpec::resolve(
        "https://github.com/example/repo",
        Some("main".to_owned()),
        false,
    )
    .unwrap();

    assert_eq!(source.kind, SourceKind::GitUrl);
    assert_eq!(
        source.cache_key(),
        "git:https://github.com/example/repo@main"
    );
    assert!(SourceSpec::resolve("https://github.com/example/repo", None, true).is_err());
}

#[test]
fn index_identity_changes_with_encoder_and_options() {
    let dir = tempfile::tempdir().unwrap();
    let source = SourceSpec::resolve(dir.path().to_string_lossy(), None, false).unwrap();
    let sparse = IndexRuntimeOptions::sparse(CacheConfig::Platform);
    let hashing = IndexRuntimeOptions::with_encoder(EncoderSpec::hashing(), CacheConfig::Platform);

    assert_ne!(
        IndexIdentity::new(source.clone(), &sparse).key(),
        IndexIdentity::new(source, &hashing).key()
    );
}

#[test]
fn request_envelopes_round_trip_as_tagged_json() {
    let dir = tempfile::tempdir().unwrap();
    let source = SourceSpec::resolve(dir.path().to_string_lossy(), None, false).unwrap();
    let request = DaemonRequestEnvelope::new(
        "r1",
        DaemonRequest::Search {
            source,
            options: IndexRuntimeOptions::sparse(CacheConfig::Platform),
            query: "login screen".to_owned(),
            search: SearchOptions::new(3).with_mode(SearchMode::Bm25).into(),
        },
    );

    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("\"type\":\"search\""));
    let decoded: DaemonRequestEnvelope = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded.protocol_version, DAEMON_PROTOCOL_VERSION);
    assert_eq!(decoded.request_id, "r1");
    assert_eq!(decoded, request);
}

#[test]
fn index_manager_reuses_and_refreshes_sources() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("auth.rs"), "fn login_screen() {}\n").unwrap();
    let source = SourceSpec::resolve(dir.path().to_string_lossy(), None, false).unwrap();
    let options = IndexRuntimeOptions::sparse(CacheConfig::Platform);
    let mut manager = IndexManager::new();

    let first_stats = manager
        .get(source.clone(), options.clone())
        .unwrap()
        .stats();
    let second_stats = manager
        .get(source.clone(), options.clone())
        .unwrap()
        .stats();

    assert_eq!(first_stats, second_stats);
    assert_eq!(manager.status().indexes.len(), 1);
    assert!(manager.clear(source, options));
    assert!(manager.status().indexes.is_empty());
}
