//! Plain Rust: `pub fn`s with simple types are callable from Python source.

pub fn dot(ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    ax * bx + ay * by
}

pub fn greet(name: &str) -> String {
    format!("hello, {name}, from Rust")
}

pub fn checked_div(a: i64, b: i64) -> Result<i64, String> {
    if b == 0 { Err("division by zero in Rust".to_string()) } else { Ok(a / b) }
}

fn private_helper() {}
