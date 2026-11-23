//! A plain Rust file. pyrock includes it as a module and exposes every
//! `pub fn` whose parameters/return are ints, floats, bool, &str/String or ().

pub fn hypot(dx: f64, dy: f64) -> f64 {
    (dx * dx + dy * dy).sqrt()
}

pub fn fib(n: u32) -> u64 {
    let (mut a, mut b) = (0u64, 1u64);
    for _ in 0..n {
        let t = a + b;
        a = b;
        b = t;
    }
    a
}

pub fn count_primes(limit: usize) -> usize {
    let mut sieve = vec![true; limit];
    let mut count = 0;
    for i in 2..limit {
        if sieve[i] {
            count += 1;
            let mut j = i * i;
            while j < limit {
                sieve[j] = false;
                j += i;
            }
        }
    }
    count
}

pub fn shout(s: &str) -> String {
    format!("{}!", s.to_uppercase())
}

/// `Err` becomes a Python `RuntimeError` with this message.
pub fn safe_div(a: i64, b: i64) -> Result<i64, String> {
    if b == 0 {
        Err(format!("cannot divide {a} by zero"))
    } else {
        Ok(a / b)
    }
}
