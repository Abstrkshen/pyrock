//! Rich Rust interop: containers, Option, tuples, maps, structs with
//! methods, callbacks and a Cargo dependency.
// pyrock: dep itoa = "1"

use std::collections::HashMap;

pub fn mean(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() { None } else { Some(xs.iter().sum::<f64>() / xs.len() as f64) }
}

pub fn scale_all(xs: &mut [f64], k: f64) {
    for x in xs.iter_mut() {
        *x *= k;
    }
}

pub fn push_twice(v: &mut Vec<i64>, x: i64) {
    v.push(x);
    v.push(x);
}

pub fn split_words(s: &str) -> Vec<String> {
    s.split_whitespace().map(|w| w.to_string()).collect()
}

pub fn min_max(xs: Vec<i32>) -> (i32, i32) {
    (*xs.iter().min().unwrap(), *xs.iter().max().unwrap())
}

pub fn histogram(words: Vec<String>) -> HashMap<String, i64> {
    let mut h = HashMap::new();
    for w in words {
        *h.entry(w).or_insert(0) += 1;
    }
    h
}

pub fn apply(xs: &[i64], f: impl Fn(i64) -> i64) -> Vec<i64> {
    xs.iter().map(|x| f(*x)).collect()
}

pub fn count_if(xs: &[f64], pred: &dyn Fn(f64) -> bool) -> usize {
    xs.iter().filter(|x| pred(**x)).count()
}

pub fn format_int(n: i64) -> String {
    let mut b = itoa::Buffer::new();
    b.format(n).to_string()
}

#[derive(Debug, Clone)]
pub struct Counter {
    pub name: String,
    pub count: i64,
}

impl Counter {
    pub fn new(name: &str) -> Self {
        Counter { name: name.to_string(), count: 0 }
    }

    pub fn incr(&mut self, by: i64) -> i64 {
        self.count += by;
        self.count
    }

    pub fn label(&self) -> String {
        format!("{}={}", self.name, self.count)
    }
}

pub fn total(counters: &Counter, other: &Counter) -> i64 {
    counters.count + other.count
}

/// Bytes in and out.
pub fn checksum(data: &[u8]) -> u32 {
    data.iter().map(|b| *b as u32).sum()
}

pub fn shout(s: &str) -> Vec<u8> {
    s.to_uppercase().into_bytes()
}
