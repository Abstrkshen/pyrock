//! Buffers: Python sequences (array.array, numpy arrays, bytes) as Rust slices.

pub fn scale_all(xs: &mut [f64], k: f64) {
    for x in xs.iter_mut() {
        *x *= k;
    }
}

pub fn total(xs: &[f64]) -> f64 {
    xs.iter().sum()
}

pub fn checksum(data: &[u8]) -> u32 {
    data.iter().map(|b| *b as u32).sum()
}

pub fn xor(data: Vec<u8>, key: u8) -> Vec<u8> {
    data.into_iter().map(|b| b ^ key).collect()
}
