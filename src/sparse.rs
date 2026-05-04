use crate::tokens::tokenize;
use crate::types::Chunk;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Bm25Index {
    postings: HashMap<String, Vec<(usize, u32)>>,
    doc_len: Vec<usize>,
    avg_doc_len: f32,
    doc_count: usize,
}

impl Bm25Index {
    pub fn build(docs: &[String]) -> Self {
        let mut postings_map: HashMap<String, HashMap<usize, u32>> = HashMap::new();
        let mut doc_len = Vec::with_capacity(docs.len());
        for (doc_id, doc) in docs.iter().enumerate() {
            let tokens = tokenize(doc);
            doc_len.push(tokens.len());
            let mut counts: HashMap<String, u32> = HashMap::new();
            for token in tokens {
                *counts.entry(token).or_default() += 1;
            }
            for (token, tf) in counts {
                postings_map.entry(token).or_default().insert(doc_id, tf);
            }
        }
        let postings = postings_map
            .into_iter()
            .map(|(term, docs)| {
                let mut docs: Vec<(usize, u32)> = docs.into_iter().collect();
                docs.sort_unstable_by_key(|(doc_id, _)| *doc_id);
                (term, docs)
            })
            .collect();
        let avg_doc_len = if doc_len.is_empty() {
            0.0
        } else {
            doc_len.iter().sum::<usize>() as f32 / doc_len.len() as f32
        };
        Self {
            postings,
            doc_len,
            avg_doc_len,
            doc_count: docs.len(),
        }
    }

    pub fn search(
        &self,
        query: &str,
        top_k: usize,
        selector: Option<&[usize]>,
    ) -> Vec<(usize, f32)> {
        let tokens = tokenize(query);
        if tokens.is_empty() || top_k == 0 {
            return Vec::new();
        }
        let allowed: Option<HashSet<usize>> = selector.map(|s| s.iter().copied().collect());
        let mut scores: HashMap<usize, f32> = HashMap::new();
        let unique_terms: HashSet<String> = tokens.into_iter().collect();
        for term in unique_terms {
            let Some(postings) = self.postings.get(&term) else {
                continue;
            };
            let df = postings.len() as f32;
            let idf = ((self.doc_count as f32 - df + 0.5) / (df + 0.5) + 1.0).ln();
            for &(doc_id, tf) in postings {
                if allowed.as_ref().is_some_and(|set| !set.contains(&doc_id)) {
                    continue;
                }
                let tf = tf as f32;
                let dl = self.doc_len[doc_id] as f32;
                let k1 = 1.5f32;
                let b = 0.75f32;
                let denom = tf + k1 * (1.0 - b + b * dl / self.avg_doc_len.max(1.0));
                let score = idf * (tf * (k1 + 1.0)) / denom;
                *scores.entry(doc_id).or_default() += score;
            }
        }
        let mut ranked: Vec<(usize, f32)> = scores
            .into_iter()
            .filter(|(_, score)| *score > 0.0)
            .collect();
        ranked.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        ranked.truncate(top_k.min(ranked.len()));
        ranked
    }
}

pub fn enrich_for_bm25(chunk: &Chunk) -> String {
    let path = Path::new(&chunk.file_path);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    let dir_text = path
        .parent()
        .map(|parent| {
            parent
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .filter(|part| part != "." && part != "/")
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" ");
    format!("{} {} {} {}", chunk.content, stem, stem, dir_text)
}
