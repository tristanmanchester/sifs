use crate::dense::DenseIndex;
use crate::model2vec::{Encoder, normalize_vector};
use crate::ranking::{
    apply_query_boost_in_place, boost_multi_chunk_files, is_symbol_query, rerank_topk_for_query,
    resolve_alpha,
};
use crate::sparse::Bm25Index;
use crate::types::{Chunk, SearchExplanation, SearchHit, SearchMode};
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
    _chunks: &[Chunk],
    top_k: usize,
    selector: Option<&[usize]>,
    explain: bool,
) -> Vec<SearchHit> {
    let encoded = model.encode(&[query.to_owned()]);
    let vector = normalize_vector(encoded.row(0).to_owned());
    semantic_index
        .query(&vector, top_k, selector)
        .into_iter()
        .enumerate()
        .map(|(rank, (idx, score))| SearchHit {
            chunk_id: idx,
            score,
            source: SearchMode::Semantic,
            explanation: explain.then(|| SearchExplanation {
                alpha: None,
                bm25_rank: None,
                bm25_score: None,
                semantic_rank: Some(rank + 1),
                semantic_score: Some(score),
                rrf_score: None,
                boosted_score: None,
                final_score: score,
            }),
        })
        .collect()
}

pub fn search_bm25(
    query: &str,
    bm25_index: &Bm25Index,
    _chunks: &[Chunk],
    top_k: usize,
    selector: Option<&[usize]>,
    explain: bool,
) -> Vec<SearchHit> {
    bm25_index
        .search(query, top_k, selector)
        .into_iter()
        .enumerate()
        .map(|(rank, (idx, score))| SearchHit {
            chunk_id: idx,
            score,
            source: SearchMode::Bm25,
            explanation: explain.then(|| SearchExplanation {
                alpha: None,
                bm25_rank: Some(rank + 1),
                bm25_score: Some(score),
                semantic_rank: None,
                semantic_score: None,
                rrf_score: None,
                boosted_score: None,
                final_score: score,
            }),
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
    symbol_mapping: Option<&HashMap<String, Vec<usize>>>,
    top_k: usize,
    alpha: Option<f32>,
    selector: Option<&[usize]>,
    explain: bool,
) -> Vec<SearchHit> {
    let alpha_weight = resolve_alpha(query, alpha);
    let candidate_count = hybrid_candidate_count(query, top_k);
    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    let encoded = model.encode(&[query.to_owned()]);
    let vector = normalize_vector(encoded.row(0).to_owned());
    #[cfg(feature = "diagnostics")]
    let encode = start.elapsed();

    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    let semantic_scores = semantic_index.query(&vector, candidate_count, selector);
    let semantic_explain = explain.then(|| ranked_evidence(&semantic_scores));
    #[cfg(feature = "diagnostics")]
    let dense = start.elapsed();

    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    let bm25_scores = bm25_index.search(query, candidate_count, selector);
    let bm25_explain = explain.then(|| ranked_evidence(&bm25_scores));
    #[cfg(feature = "diagnostics")]
    let bm25 = start.elapsed();

    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    let mut combined = HashMap::with_capacity(semantic_scores.len() + bm25_scores.len());
    add_rrf_scores(&mut combined, &semantic_scores, alpha_weight);
    add_rrf_scores(&mut combined, &bm25_scores, 1.0 - alpha_weight);
    add_retriever_agreement_scores(&mut combined, &semantic_scores, &bm25_scores);
    add_symbol_candidate_scores(&mut combined, query, symbol_mapping, selector);
    add_file_card_candidate_scores(&mut combined, query, chunks, file_mapping, selector);
    let rrf_explain = explain.then(|| combined.clone());
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
    let boosted_explain = explain.then(|| boosted.clone());
    #[cfg(feature = "diagnostics")]
    let query_boost = start.elapsed();

    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    let ranked = rerank_topk_for_query(&boosted, chunks, top_k, alpha_weight < 1.0, query);
    #[cfg(feature = "diagnostics")]
    let rerank = start.elapsed();

    #[cfg(feature = "diagnostics")]
    let start = Instant::now();
    let results = ranked
        .into_iter()
        .map(|(idx, score)| SearchHit {
            chunk_id: idx,
            score,
            source: SearchMode::Hybrid,
            explanation: explain.then(|| SearchExplanation {
                alpha: Some(alpha_weight),
                bm25_rank: bm25_explain
                    .as_ref()
                    .and_then(|scores| scores.get(&idx).map(|evidence| evidence.rank)),
                bm25_score: bm25_explain
                    .as_ref()
                    .and_then(|scores| scores.get(&idx).map(|evidence| evidence.score)),
                semantic_rank: semantic_explain
                    .as_ref()
                    .and_then(|scores| scores.get(&idx).map(|evidence| evidence.rank)),
                semantic_score: semantic_explain
                    .as_ref()
                    .and_then(|scores| scores.get(&idx).map(|evidence| evidence.score)),
                rrf_score: rrf_explain
                    .as_ref()
                    .and_then(|scores| scores.get(&idx).copied()),
                boosted_score: boosted_explain
                    .as_ref()
                    .and_then(|scores| scores.get(&idx).copied()),
                final_score: score,
            }),
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

fn add_symbol_candidate_scores<S: std::hash::BuildHasher>(
    combined: &mut HashMap<usize, f32, S>,
    query: &str,
    symbol_mapping: Option<&HashMap<String, Vec<usize>>>,
    selector: Option<&[usize]>,
) {
    if !is_symbol_query(query) && !has_embedded_symbol_query_term(query) {
        return;
    }
    let Some(symbol_mapping) = symbol_mapping else {
        return;
    };
    let selector_set = selector.map(|values| {
        values
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
    });
    let mut injected = 0usize;
    for term in symbol_query_terms(query) {
        let Some(postings) = symbol_mapping.get(&term) else {
            continue;
        };
        for (rank, idx) in postings.iter().copied().enumerate() {
            if selector_set
                .as_ref()
                .is_some_and(|selector| !selector.contains(&idx))
            {
                continue;
            }
            *combined.entry(idx).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
            injected += 1;
            if injected >= 64 {
                return;
            }
        }
    }
}

fn has_embedded_symbol_query_term(query: &str) -> bool {
    query.split_whitespace().any(|part| {
        let trimmed =
            part.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$');
        trimmed.len() >= 3
            && trimmed
                .as_bytes()
                .windows(2)
                .any(|pair| pair[0].is_ascii_lowercase() && pair[1].is_ascii_uppercase())
    })
}

fn symbol_query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for raw in query.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$')) {
        if raw.len() < 2 {
            continue;
        }
        let lowered = raw.to_ascii_lowercase();
        if !terms.contains(&lowered) {
            terms.push(lowered);
        }
        for part in crate::tokens::split_identifier(raw) {
            if part.len() >= 2 && !terms.contains(&part) {
                terms.push(part);
            }
        }
    }
    terms
}

fn add_file_card_candidate_scores<S: std::hash::BuildHasher>(
    combined: &mut HashMap<usize, f32, S>,
    query: &str,
    chunks: &[Chunk],
    file_mapping: Option<&HashMap<String, Vec<usize>>>,
    selector: Option<&[usize]>,
) {
    if !looks_file_card_query(query)
        && !(looks_architectural_or_natural_language(query) && chunks.len() <= 1_500)
    {
        return;
    }
    let Some(file_mapping) = file_mapping else {
        return;
    };
    let query_terms = crate::tokens::tokenize(query)
        .into_iter()
        .filter(|term| term.len() >= 3)
        .collect::<std::collections::HashSet<_>>();
    if query_terms.is_empty() {
        return;
    }
    let selector_set = selector.map(|values| {
        values
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
    });
    let mut file_scores = Vec::new();
    for (file_path, chunk_ids) in file_mapping {
        let overlap = file_card_overlap(&query_terms, file_path, chunk_ids, chunks);
        if overlap == 0 {
            continue;
        }
        let Some(first_allowed_chunk) = chunk_ids.iter().copied().find(|idx| {
            selector_set
                .as_ref()
                .is_none_or(|selector| selector.contains(idx))
        }) else {
            continue;
        };
        file_scores.push((first_allowed_chunk, overlap as f32));
    }
    file_scores.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (rank, (chunk_id, score)) in file_scores.into_iter().take(32).enumerate() {
        *combined.entry(chunk_id).or_default() += 0.5 * score / (RRF_K + rank as f32 + 1.0);
    }
}

fn file_card_overlap(
    query_terms: &std::collections::HashSet<String>,
    file_path: &str,
    chunk_ids: &[usize],
    chunks: &[Chunk],
) -> usize {
    let mut overlap = crate::tokens::tokenize(file_path)
        .into_iter()
        .filter(|term| query_terms.contains(term))
        .collect::<std::collections::HashSet<_>>();
    if overlap.is_empty() {
        return 0;
    }
    if overlap.len() == query_terms.len() {
        return overlap.len();
    }
    for chunk_id in chunk_ids {
        let Some(chunk) = chunks.get(*chunk_id) else {
            continue;
        };
        if let Some(language) = &chunk.language {
            overlap.extend(
                crate::tokens::tokenize(language)
                    .into_iter()
                    .filter(|term| query_terms.contains(term)),
            );
        }
        for symbol in &chunk.symbols {
            overlap.extend(
                crate::tokens::tokenize(&symbol.name)
                    .into_iter()
                    .filter(|term| query_terms.contains(term)),
            );
            overlap.extend(
                crate::tokens::tokenize(&symbol.kind)
                    .into_iter()
                    .filter(|term| query_terms.contains(term)),
            );
        }
        for breadcrumb in &chunk.breadcrumbs {
            overlap.extend(
                crate::tokens::tokenize(breadcrumb)
                    .into_iter()
                    .filter(|term| query_terms.contains(term)),
            );
        }
        if overlap.len() == query_terms.len() {
            break;
        }
    }
    overlap.len()
}

fn hybrid_candidate_count(query: &str, top_k: usize) -> usize {
    let top_k = top_k.max(1);
    if is_symbol_query(query) {
        top_k.saturating_mul(12).max(120)
    } else if looks_architectural_or_natural_language(query) {
        top_k.saturating_mul(50).max(500)
    } else {
        top_k.saturating_mul(20).max(200)
    }
}

fn looks_architectural_or_natural_language(query: &str) -> bool {
    let lowered = query.trim().to_lowercase();
    lowered.ends_with('?')
        || lowered.starts_with("how ")
        || lowered.starts_with("what ")
        || lowered.starts_with("where ")
        || lowered.starts_with("when ")
        || lowered.starts_with("why ")
        || lowered.starts_with("which ")
        || lowered.starts_with("who ")
        || lowered.contains(" architecture")
        || lowered.contains(" design")
        || lowered.contains(" flow")
        || lowered.contains(" lifecycle")
        || lowered.contains(" pipeline")
}

fn looks_file_card_query(query: &str) -> bool {
    let lowered = query.trim().to_lowercase();
    lowered.contains(" architecture")
        || lowered.contains(" design")
        || lowered.contains(" flow")
        || lowered.contains(" lifecycle")
        || lowered.contains(" pipeline")
        || lowered.contains(" module")
        || lowered.contains(" subsystem")
        || lowered.contains(" package")
        || lowered.contains(" layer")
        || lowered.contains(" entry point")
        || lowered.contains(" where ")
        || lowered.starts_with("where ")
}

#[derive(Clone, Copy)]
struct RankEvidence {
    rank: usize,
    score: f32,
}

fn ranked_evidence(scores: &[(usize, f32)]) -> HashMap<usize, RankEvidence> {
    scores
        .iter()
        .enumerate()
        .map(|(rank, (idx, score))| {
            (
                *idx,
                RankEvidence {
                    rank: rank + 1,
                    score: *score,
                },
            )
        })
        .collect()
}

fn add_rrf_scores<S: std::hash::BuildHasher>(
    combined: &mut HashMap<usize, f32, S>,
    ranked: &[(usize, f32)],
    weight: f32,
) {
    for (rank, (id, _)) in ranked.iter().enumerate() {
        *combined.entry(*id).or_default() += weight / (RRF_K + rank as f32 + 1.0);
    }
}

fn add_retriever_agreement_scores<S: std::hash::BuildHasher>(
    combined: &mut HashMap<usize, f32, S>,
    semantic_scores: &[(usize, f32)],
    bm25_scores: &[(usize, f32)],
) {
    let bm25_ranks = bm25_scores
        .iter()
        .take(50)
        .enumerate()
        .map(|(rank, (id, _))| (*id, rank))
        .collect::<HashMap<_, _>>();
    for (semantic_rank, (id, _)) in semantic_scores.iter().take(50).enumerate() {
        let Some(&bm25_rank) = bm25_ranks.get(id) else {
            continue;
        };
        let best_rank = semantic_rank.min(bm25_rank) as f32;
        let worst_rank = semantic_rank.max(bm25_rank) as f32;
        let agreement = 0.35 / (RRF_K + best_rank + 1.0);
        let balance = 1.0 / (1.0 + (worst_rank - best_rank) / 25.0);
        *combined.entry(*id).or_default() += agreement * balance;
        if semantic_rank < 5 && bm25_rank < 5 {
            let strong_agreement = 1.5 / (RRF_K + best_rank + 1.0);
            *combined.entry(*id).or_default() += strong_agreement * balance;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        add_file_card_candidate_scores, add_retriever_agreement_scores, add_rrf_scores,
        add_symbol_candidate_scores, hybrid_candidate_count, search_bm25, search_hybrid,
        search_semantic,
    };
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
            symbols: Vec::new(),
            breadcrumbs: Vec::new(),
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

        let semantic = search_semantic("parse session", &model, &dense, &chunks, 1, None, false);
        let bm25 = search_bm25("parse_session_token", &sparse, &chunks, 1, None, false);
        let hybrid = search_hybrid(
            "parse session",
            &model,
            &dense,
            &sparse,
            &chunks,
            None,
            None,
            1,
            None,
            None,
            false,
        );

        assert_eq!(semantic[0].source, SearchMode::Semantic);
        assert_eq!(bm25[0].source, SearchMode::Bm25);
        assert_eq!(hybrid[0].source, SearchMode::Hybrid);
    }

    #[test]
    fn rrf_scores_prioritize_higher_ranked_items() {
        let mut normalized = HashMap::new();
        add_rrf_scores(&mut normalized, &[(1, 0.9), (2, 0.1)], 1.0);

        assert!(normalized[&1] > normalized[&2]);
    }

    #[test]
    fn retriever_agreement_adds_balanced_top_rank_signal() {
        let mut combined = HashMap::from([(1usize, 0.01), (2usize, 0.01), (3usize, 0.01)]);

        add_retriever_agreement_scores(
            &mut combined,
            &[(1, 0.9), (2, 0.8), (3, 0.7)],
            &[(1, 12.0), (3, 11.0), (2, 10.0)],
        );

        assert!(combined[&1] > combined[&2]);
        assert!(combined[&3] > 0.01);
    }

    #[test]
    fn retriever_agreement_rewards_top_rank_consensus() {
        let mut combined = HashMap::from([(1usize, 0.01), (2usize, 0.04)]);

        add_retriever_agreement_scores(&mut combined, &[(1, 0.9)], &[(1, 12.0)]);

        assert!(combined[&1] > combined[&2]);
    }

    #[test]
    fn hybrid_candidate_pool_expands_for_natural_language() {
        assert_eq!(hybrid_candidate_count("TokenManager", 10), 120);
        assert_eq!(hybrid_candidate_count("token manager", 10), 200);
        assert_eq!(
            hybrid_candidate_count("how request lifecycle works", 10),
            500
        );
    }

    #[test]
    fn symbol_candidates_are_injected_from_exact_postings() {
        let mut symbol_mapping = HashMap::new();
        symbol_mapping.insert("tokenmanager".to_owned(), vec![42]);
        let mut combined = HashMap::new();

        add_symbol_candidate_scores(
            &mut combined,
            "where TokenManager is used",
            Some(&symbol_mapping),
            None,
        );

        assert!(combined.contains_key(&42));
    }

    #[test]
    fn symbol_candidates_skip_plain_prose_queries() {
        let mut symbol_mapping = HashMap::new();
        symbol_mapping.insert("request".to_owned(), vec![42]);
        let mut combined = HashMap::new();

        add_symbol_candidate_scores(
            &mut combined,
            "request validation and error handling",
            Some(&symbol_mapping),
            None,
        );

        assert!(combined.is_empty());
    }

    #[test]
    fn file_card_candidates_are_injected_for_architecture_queries() {
        let chunks = vec![
            chunk("fn unrelated() {}", "src/other.rs"),
            chunk(
                "pub struct Router {}\npub fn lifecycle() {}",
                "src/http/router.rs",
            ),
        ];
        let file_mapping = HashMap::from([
            ("src/other.rs".to_owned(), vec![0]),
            ("src/http/router.rs".to_owned(), vec![1]),
        ]);
        let mut combined = HashMap::new();

        add_file_card_candidate_scores(
            &mut combined,
            "how request router lifecycle works",
            &chunks,
            Some(&file_mapping),
            None,
        );

        assert!(combined.contains_key(&1));
    }
}
