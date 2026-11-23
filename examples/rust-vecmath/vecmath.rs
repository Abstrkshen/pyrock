pub fn dot(ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    ax * bx + ay * by
}

pub fn length(x: f64, y: f64) -> f64 {
    (x * x + y * y).sqrt()
}

pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

pub fn label(name: &str) -> String {
    format!("rust {name}")
}
