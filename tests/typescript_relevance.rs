use serde::Deserialize;
use sifs::{EncoderSpec, SearchMode, SearchOptions, SifsIndex};
use std::path::Path;

#[derive(Debug, Deserialize)]
struct GoldenQuery {
    query: String,
    expected_path: String,
    max_rank: usize,
}

#[test]
fn typescript_mini_corpus_keeps_expected_surfaces_findable() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts-mini-corpus");
    let golden: Vec<GoldenQuery> =
        serde_json::from_str(&std::fs::read_to_string(corpus.join("golden.json")).unwrap())
            .unwrap();
    let index =
        SifsIndex::from_path_with_encoder_spec(&corpus, EncoderSpec::hashing(), None, None, false)
            .unwrap();

    for case in golden {
        let results = index
            .search_with(
                &case.query,
                &SearchOptions::new(10)
                    .with_mode(SearchMode::Hybrid)
                    .with_cache(false),
            )
            .unwrap();
        let top_paths: Vec<_> = results
            .iter()
            .map(|result| result.chunk.file_path.as_str())
            .collect();

        assert!(
            top_paths
                .iter()
                .take(case.max_rank)
                .any(|path| path.ends_with(&case.expected_path)),
            "query {:?} expected {:?} in top {}, got {:?}",
            case.query,
            case.expected_path,
            case.max_rank,
            top_paths
        );
    }
}
