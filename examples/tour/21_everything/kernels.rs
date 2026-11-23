/// Map a value in [0, 1) to a smooth, zero-centred push.
pub fn smooth(u: f64) -> f64 {
    let c = u - 0.5;
    c * c * c * 4.0
}
