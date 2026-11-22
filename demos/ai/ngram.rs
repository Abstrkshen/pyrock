//! A bigram language model over token ids.
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Bigram {
    pub vocab: i64,
    counts: HashMap<(i64, i64), f64>,
    totals: HashMap<i64, f64>,
}

impl Bigram {
    pub fn new(vocab: i64) -> Bigram {
        Bigram { vocab, counts: HashMap::new(), totals: HashMap::new() }
    }

    /// Count every adjacent pair of `ids`.
    pub fn train(&mut self, ids: &[i64]) {
        for w in ids.windows(2) {
            *self.counts.entry((w[0], w[1])).or_insert(0.0) += 1.0;
            *self.totals.entry(w[0]).or_insert(0.0) += 1.0;
        }
    }

    /// Log-probabilities of the next token after `prev` (add-k smoothing).
    pub fn logits(&self, prev: i64, k: f64) -> Vec<f64> {
        let total = self.totals.get(&prev).copied().unwrap_or(0.0) + k * self.vocab as f64;
        (0..self.vocab).map(|t| ((self.counts.get(&(prev, t)).copied().unwrap_or(0.0) + k) / total).ln()).collect()
    }

    pub fn pairs(&self) -> i64 {
        self.counts.len() as i64
    }
}

/// Mean negative log-likelihood of `ids` under the model.
pub fn perplexity(model: &Bigram, ids: &[i64], k: f64) -> f64 {
    let mut nll = 0.0;
    for w in ids.windows(2) {
        nll -= model.logits(w[0], k)[w[1] as usize];
    }
    (nll / (ids.len() - 1) as f64).exp()
}
