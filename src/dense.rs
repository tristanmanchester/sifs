use ndarray::{Array1, Array2, Axis};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DenseIndex {
    vectors: Array2<f32>,
}

impl DenseIndex {
    pub fn new(mut vectors: Array2<f32>) -> Self {
        for mut row in vectors.axis_iter_mut(Axis(0)) {
            let norm = row.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 1e-8 {
                row.mapv_inplace(|v| v / norm);
            }
        }
        Self { vectors }
    }

    pub fn len(&self) -> usize {
        self.vectors.shape()[0]
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn query(
        &self,
        vector: &Array1<f32>,
        k: usize,
        selector: Option<&[usize]>,
    ) -> Vec<(usize, f32)> {
        if k == 0 || self.is_empty() {
            return Vec::new();
        }
        let candidates: Vec<usize> = selector
            .map(|s| s.to_vec())
            .unwrap_or_else(|| (0..self.len()).collect());
        let mut scores: Vec<(usize, f32)> = candidates
            .par_iter()
            .map(|&idx| {
                let row = self.vectors.row(idx);
                let score = row
                    .iter()
                    .zip(vector.iter())
                    .map(|(a, b)| a * b)
                    .sum::<f32>();
                (idx, score)
            })
            .collect();
        truncate_top_k(&mut scores, k);
        scores
    }
}

pub(crate) fn truncate_top_k(scores: &mut Vec<(usize, f32)>, k: usize) {
    if scores.len() <= k {
        scores.sort_unstable_by(desc_score);
        return;
    }
    scores.select_nth_unstable_by(k, desc_score);
    scores.truncate(k);
    scores.sort_unstable_by(desc_score);
}

fn desc_score(a: &(usize, f32), b: &(usize, f32)) -> Ordering {
    b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal)
}
