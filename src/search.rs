use crate::dense::DenseIndex;
use crate::model2vec::{Encoder, normalize_vector};
use crate::ranking::{apply_query_boost, boost_multi_chunk_files, rerank_topk, resolve_alpha};
use crate::sparse::Bm25Index;
use crate::types::{Chunk, SearchMode, SearchResult};
use std::collections::HashMap;

const RRF_K: f32 = 60.0;

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

pub fn search_hybrid(
    query: &str,
    model: &dyn Encoder,
    semantic_index: &DenseIndex,
    bm25_index: &Bm25Index,
    chunks: &[Chunk],
    top_k: usize,
    alpha: Option<f32>,
    selector: Option<&[usize]>,
) -> Vec<SearchResult> {
    let alpha_weight = resolve_alpha(query, alpha);
    let candidate_count = top_k.saturating_mul(9).max(top_k).max(1);
    let encoded = model.encode(&[query.to_owned()]);
    let vector = normalize_vector(encoded.row(0).to_owned());
    let semantic_scores: HashMap<usize, f32> = semantic_index
        .query(&vector, candidate_count, selector)
        .into_iter()
        .collect();
    let bm25_scores: HashMap<usize, f32> = bm25_index
        .search(query, candidate_count, selector)
        .into_iter()
        .collect();
    let normalized_semantic = rrf_scores(&semantic_scores);
    let normalized_bm25 = rrf_scores(&bm25_scores);
    let mut combined = HashMap::new();
    for chunk_id in normalized_semantic.keys().chain(normalized_bm25.keys()) {
        let score = alpha_weight * normalized_semantic.get(chunk_id).copied().unwrap_or(0.0)
            + (1.0 - alpha_weight) * normalized_bm25.get(chunk_id).copied().unwrap_or(0.0);
        combined.insert(*chunk_id, score);
    }
    boost_multi_chunk_files(&mut combined, chunks);
    let boosted = apply_query_boost(&combined, query, chunks);
    rerank_topk(&boosted, chunks, top_k, alpha_weight < 1.0)
        .into_iter()
        .map(|(idx, score)| SearchResult {
            chunk: chunks[idx].clone(),
            score,
            source: SearchMode::Hybrid,
        })
        .collect()
}

fn rrf_scores(scores: &HashMap<usize, f32>) -> HashMap<usize, f32> {
    let mut ranked: Vec<(usize, f32)> = scores.iter().map(|(&id, &score)| (id, score)).collect();
    ranked.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked
        .into_iter()
        .enumerate()
        .map(|(rank, (id, _))| (id, 1.0 / (RRF_K + rank as f32 + 1.0)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{rrf_scores, search_bm25, search_hybrid, search_semantic};
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
        let scores = HashMap::from([(1, 0.9), (2, 0.1)]);
        let normalized = rrf_scores(&scores);

        assert!(normalized[&1] > normalized[&2]);
    }
}
