use crate::dense::DenseIndex;
use crate::model2vec::{Encoder, normalize_vector};
use crate::ranking::{
    apply_query_boost_in_place, boost_multi_chunk_files, rerank_topk, resolve_alpha,
};
use crate::sparse::Bm25Index;
use crate::types::{Chunk, SearchMode, SearchResult};
use std::collections::HashMap;
#[cfg(feature = "diagnostics")]
use std::sync::{Mutex, OnceLock};
#[cfg(feature = "diagnostics")]
use std::time::Duration;
#[cfg(feature = "diagnostics")]
use std::time::Instant;

const RRF_K: f32 = 60.0;

#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy, Debug, Default)]
pub struct HybridTiming {
    pub queries: usize,
    pub encode: Duration,
    pub dense: Duration,
    pub bm25: Duration,
    pub fuse: Duration,
    pub file_boost: Duration,
    pub query_boost: Duration,
    pub rerank: Duration,
    pub collect: Duration,
}

#[cfg(feature = "diagnostics")]
static HYBRID_TIMING: OnceLock<Mutex<HybridTiming>> = OnceLock::new();

#[cfg(feature = "diagnostics")]
fn timing() -> &'static Mutex<HybridTiming> {
    HYBRID_TIMING.get_or_init(|| Mutex::new(HybridTiming::default()))
}

#[cfg(feature = "diagnostics")]
pub fn reset_hybrid_timing() {
    if let Ok(mut timing) = timing().lock() {
        *timing = HybridTiming::default();
    }
}

#[cfg(feature = "diagnostics")]
pub fn hybrid_timing() -> HybridTiming {
    timing().lock().map(|timing| *timing).unwrap_or_default()
}

pub fn search_semantic(
    query: &str,
    model: &dyn Encoder,
    semantic_index: &DenseIndex,
    chunks: &[Chunk],
    top_k: usize,
    selector: Option<&[usize]>,
) -> Vec<SearchResult> {
    let encoded = model.encode(&[query.to_owned()]);
    let vector = normalize_vector(encoded.row(0).to_owned());
    semantic_index
        .query(&vector, top_k, selector)
        .into_iter()
        .map(|(idx, score)| SearchResult {
            chunk: chunks[idx].clone(),
            score,
            source: SearchMode::Semantic,
        })
        .collect()
}

pub fn search_bm25(
    query: &str,
    bm25_index: &Bm25Index,
    chunks: &[Chunk],
    top_k: usize,
    selector: Option<&[usize]>,
) -> Vec<SearchResult> {
    bm25_index
        .search(query, top_k, selector)
        .into_iter()
        .map(|(idx, score)| SearchResult {
            chunk: chunks[idx].clone(),
            score,
            source: SearchMode::Bm25,
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn search_hybrid(
    query: &str,
    model: &dyn Encoder,
    semantic_index: &DenseIndex,
    bm25_index: &Bm25Index,
    chunks: &[Chunk],
    file_mapping: Option<&HashMap<String, Vec<usize>>>,
    top_k: usize,
    alpha: Option<f32>,
    selector: Option<&[usize]>,
) -> Vec<SearchResult> {
    let alpha_weight = resolve_alpha(query, alpha);
    let candidate_count = top_k.saturating_mul(9).max(top_k).max(1);
    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    let encoded = model.encode(&[query.to_owned()]);
    let vector = normalize_vector(encoded.row(0).to_owned());
    #[cfg(feature = "diagnostics")]
    let encode = start.elapsed();

    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    let semantic_scores = semantic_index.query(&vector, candidate_count, selector);
    #[cfg(feature = "diagnostics")]
    let dense = start.elapsed();

    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    let bm25_scores = bm25_index.search(query, candidate_count, selector);
    #[cfg(feature = "diagnostics")]
    let bm25 = start.elapsed();

    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    let mut combined = HashMap::with_capacity(semantic_scores.len() + bm25_scores.len());
    add_rrf_scores(&mut combined, semantic_scores, alpha_weight);
    add_rrf_scores(&mut combined, bm25_scores, 1.0 - alpha_weight);
    #[cfg(feature = "diagnostics")]
    let fuse = start.elapsed();

    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    boost_multi_chunk_files(&mut combined, chunks);
    #[cfg(feature = "diagnostics")]
    let file_boost = start.elapsed();

    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    let boosted = apply_query_boost_in_place(combined, query, chunks, file_mapping);
    #[cfg(feature = "diagnostics")]
    let query_boost = start.elapsed();

    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    let ranked = rerank_topk(&boosted, chunks, top_k, alpha_weight < 1.0);
    #[cfg(feature = "diagnostics")]
    let rerank = start.elapsed();

    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    let results = ranked
        .into_iter()
        .map(|(idx, score)| SearchResult {
            chunk: chunks[idx].clone(),
            score,
            source: SearchMode::Hybrid,
        })
        .collect();
    #[cfg(feature = "diagnostics")]
    {
        let collect = start.elapsed();
        if let Ok(mut timing) = timing().lock() {
            timing.queries += 1;
            timing.encode += encode;
            timing.dense += dense;
            timing.bm25 += bm25;
            timing.fuse += fuse;
            timing.file_boost += file_boost;
            timing.query_boost += query_boost;
            timing.rerank += rerank;
            timing.collect += collect;
        }
    }
    results
}

fn add_rrf_scores<S: std::hash::BuildHasher>(
    combined: &mut HashMap<usize, f32, S>,
    ranked: Vec<(usize, f32)>,
    weight: f32,
) {
    for (rank, (id, _)) in ranked.into_iter().enumerate() {
        *combined.entry(id).or_default() += weight / (RRF_K + rank as f32 + 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::{add_rrf_scores, search_bm25, search_hybrid, search_semantic};
    use crate::dense::DenseIndex;
    use crate::model2vec::Encoder;
    use crate::sparse::Bm25Index;
    use crate::types::{Chunk, SearchMode};
    use ndarray::{Array2, s};
    use std::collections::HashMap;

    struct TestEncoder;

    impl Encoder for TestEncoder {
        fn dim(&self) -> usize {
            2
        }

        fn encode(&self, texts: &[String]) -> Array2<f32> {
            let mut values = Array2::zeros((texts.len(), 2));
            for (idx, text) in texts.iter().enumerate() {
                if text.contains("parse") || text.contains("session") {
                    values
                        .slice_mut(s![idx, ..])
                        .assign(&ndarray::array![1.0, 0.0]);
                } else {
                    values
                        .slice_mut(s![idx, ..])
                        .assign(&ndarray::array![0.0, 1.0]);
                }
            }
            values
        }
    }

    fn chunk(content: &str, file_path: &str) -> Chunk {
        Chunk {
            content: content.to_owned(),
            file_path: file_path.to_owned(),
            start_line: 1,
            end_line: 1,
            language: Some("rust".to_owned()),
        }
    }

    #[test]
    fn search_helpers_report_their_source_modes() {
        let chunks = vec![
            chunk("fn parse_session_token() {}", "src/auth.rs"),
            chunk("fn draw_chart() {}", "src/chart.rs"),
        ];
        let model = TestEncoder;
        let vectors = model.encode(
            &chunks
                .iter()
                .map(|chunk| chunk.content.clone())
                .collect::<Vec<_>>(),
        );
        let dense = DenseIndex::new(vectors);
        let sparse = Bm25Index::build_from_chunks(&chunks);

        let semantic = search_semantic("parse session", &model, &dense, &chunks, 1, None);
        let bm25 = search_bm25("parse_session_token", &sparse, &chunks, 1, None);
        let hybrid = search_hybrid(
            "parse session",
            &model,
            &dense,
            &sparse,
            &chunks,
            None,
            1,
            None,
            None,
        );

        assert_eq!(semantic[0].source, SearchMode::Semantic);
        assert_eq!(bm25[0].source, SearchMode::Bm25);
        assert_eq!(hybrid[0].source, SearchMode::Hybrid);
    }

    #[test]
    fn rrf_scores_prioritize_higher_ranked_items() {
        let mut normalized = HashMap::new();
        add_rrf_scores(&mut normalized, vec![(1, 0.9), (2, 0.1)], 1.0);

        assert!(normalized[&1] > normalized[&2]);
    }
}
