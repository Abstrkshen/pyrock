//! The `math` module, natively, with CPython's results and errors
//! (`ValueError: math domain error`, `OverflowError: math range error`).

use crate::exc::{Exc, PyResult};

fn domain() -> Exc {
    Exc::new("ValueError", "math domain error")
}

fn range() -> Exc {
    Exc::new("OverflowError", "math range error")
}

/// CPython's `math_1`: a NaN result from a non-NaN argument is a domain
/// error; an infinite result from a finite argument is a range error (when
/// the function can overflow) or a domain error.
#[inline]
fn m1(x: f64, r: f64, can_overflow: bool) -> PyResult<f64> {
    if r.is_nan() && !x.is_nan() {
        return Err(domain());
    }
    if r.is_infinite() && x.is_finite() {
        return Err(if can_overflow { range() } else { domain() });
    }
    Ok(r)
}

/// CPython's `math_2` for two-argument functions.
#[inline]
fn m2(x: f64, y: f64, r: f64) -> PyResult<f64> {
    if r.is_nan() && !x.is_nan() && !y.is_nan() {
        return Err(domain());
    }
    if r.is_infinite() && x.is_finite() && y.is_finite() {
        return Err(range());
    }
    Ok(r)
}

pub fn sqrt(x: f64) -> PyResult<f64> {
    m1(x, x.sqrt(), false)
}
pub fn exp(x: f64) -> PyResult<f64> {
    m1(x, x.exp(), true)
}
pub fn exp2(x: f64) -> PyResult<f64> {
    m1(x, x.exp2(), true)
}
pub fn expm1(x: f64) -> PyResult<f64> {
    m1(x, x.exp_m1(), true)
}
pub fn sin(x: f64) -> PyResult<f64> {
    m1(x, x.sin(), false)
}
pub fn cos(x: f64) -> PyResult<f64> {
    m1(x, x.cos(), false)
}
pub fn tan(x: f64) -> PyResult<f64> {
    m1(x, x.tan(), false)
}
pub fn asin(x: f64) -> PyResult<f64> {
    m1(x, x.asin(), false)
}
pub fn acos(x: f64) -> PyResult<f64> {
    m1(x, x.acos(), false)
}
pub fn atan(x: f64) -> PyResult<f64> {
    m1(x, x.atan(), false)
}
pub fn sinh(x: f64) -> PyResult<f64> {
    m1(x, x.sinh(), true)
}
pub fn cosh(x: f64) -> PyResult<f64> {
    m1(x, x.cosh(), true)
}
pub fn tanh(x: f64) -> PyResult<f64> {
    m1(x, x.tanh(), false)
}
pub fn asinh(x: f64) -> PyResult<f64> {
    m1(x, x.asinh(), false)
}
pub fn acosh(x: f64) -> PyResult<f64> {
    m1(x, x.acosh(), false)
}
pub fn atanh(x: f64) -> PyResult<f64> {
    if x.abs() >= 1.0 {
        return Err(domain());
    }
    m1(x, x.atanh(), false)
}
pub fn cbrt(x: f64) -> PyResult<f64> {
    Ok(x.cbrt())
}
pub fn fabs(x: f64) -> PyResult<f64> {
    Ok(x.abs())
}
pub fn degrees(x: f64) -> PyResult<f64> {
    m1(x, x.to_degrees(), true)
}
pub fn radians(x: f64) -> PyResult<f64> {
    Ok(x.to_radians())
}

pub fn log(x: f64) -> PyResult<f64> {
    if x.is_nan() {
        return Ok(x);
    }
    if x <= 0.0 {
        return Err(domain());
    }
    Ok(x.ln())
}
/// `log(x, base)`
pub fn log_base(x: f64, base: f64) -> PyResult<f64> {
    let d = log(base)?;
    if d == 0.0 {
        return Err(Exc::zero_div("float division by zero"));
    }
    Ok(log(x)? / d)
}
pub fn log2(x: f64) -> PyResult<f64> {
    if x <= 0.0 {
        return Err(domain());
    }
    Ok(x.log2())
}
pub fn log10(x: f64) -> PyResult<f64> {
    if x <= 0.0 {
        return Err(domain());
    }
    Ok(x.log10())
}
pub fn log1p(x: f64) -> PyResult<f64> {
    if x <= -1.0 {
        return Err(domain());
    }
    Ok(x.ln_1p())
}

pub fn atan2(y: f64, x: f64) -> PyResult<f64> {
    Ok(y.atan2(x))
}
pub fn copysign(x: f64, y: f64) -> PyResult<f64> {
    Ok(x.copysign(y))
}
pub fn fmod(x: f64, y: f64) -> PyResult<f64> {
    if y.is_infinite() && x.is_finite() {
        return Ok(x);
    }
    m2(x, y, x % y)
}
pub fn remainder(x: f64, y: f64) -> PyResult<f64> {
    if x.is_nan() || y.is_nan() {
        return Ok(f64::NAN);
    }
    if x.is_infinite() || y == 0.0 {
        return Err(domain());
    }
    if y.is_infinite() {
        return Ok(x);
    }
    let n = (x / y).round();
    // Ties go to the even multiple.
    let n = if ((x / y) - (x / y).trunc()).abs() == 0.5 { 2.0 * ((x / y) / 2.0).round() } else { n };
    Ok(x - n * y)
}
pub fn pow(x: f64, y: f64) -> PyResult<f64> {
    if x == 0.0 && y < 0.0 && y.is_finite() {
        return Err(domain());
    }
    if x < 0.0 && x.is_finite() && y.is_finite() && y.fract() != 0.0 {
        return Err(domain());
    }
    m2(x, y, x.powf(y))
}
pub fn ldexp(x: f64, i: i64) -> PyResult<f64> {
    let r = x * 2f64.powi(i.clamp(-2000, 2000) as i32);
    m1(x, r, true)
}
pub fn hypot(xs: &[f64]) -> PyResult<f64> {
    if xs.iter().any(|x| x.is_infinite()) {
        return Ok(f64::INFINITY);
    }
    let m = xs.iter().fold(0.0f64, |a, x| a.max(x.abs()));
    if m == 0.0 || m.is_nan() {
        return Ok(m);
    }
    Ok(m * xs.iter().map(|x| (x / m) * (x / m)).sum::<f64>().sqrt())
}

fn to_int(x: f64) -> PyResult<i64> {
    if x.is_nan() {
        return Err(Exc::new("ValueError", "cannot convert float NaN to integer"));
    }
    if x.is_infinite() {
        return Err(Exc::new("OverflowError", "cannot convert float infinity to integer"));
    }
    if x < -9.223_372_036_854_776e18 || x >= 9.223_372_036_854_776e18 {
        return Err(Exc::overflow());
    }
    Ok(x as i64)
}
pub fn floor(x: f64) -> PyResult<i64> {
    to_int(x.floor())
}
pub fn ceil(x: f64) -> PyResult<i64> {
    to_int(x.ceil())
}
pub fn trunc(x: f64) -> PyResult<i64> {
    to_int(x.trunc())
}

pub fn isnan(x: f64) -> PyResult<bool> {
    Ok(x.is_nan())
}
pub fn isinf(x: f64) -> PyResult<bool> {
    Ok(x.is_infinite())
}
pub fn isfinite(x: f64) -> PyResult<bool> {
    Ok(x.is_finite())
}
pub fn isclose(a: f64, b: f64, rel_tol: f64, abs_tol: f64) -> PyResult<bool> {
    if rel_tol < 0.0 || abs_tol < 0.0 {
        return Err(Exc::new("ValueError", "tolerances must be non-negative"));
    }
    if a == b {
        return Ok(true);
    }
    if a.is_infinite() || b.is_infinite() {
        return Ok(false);
    }
    let d = (b - a).abs();
    Ok(d <= (rel_tol * b).abs() || d <= (rel_tol * a).abs() || d <= abs_tol)
}

fn gcd2(a: i64, b: i64) -> i64 {
    let (mut a, mut b) = (a.unsigned_abs(), b.unsigned_abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a as i64
}
pub fn gcd(xs: &[i64]) -> PyResult<i64> {
    Ok(xs.iter().fold(0, |a, x| gcd2(a, *x)))
}
pub fn lcm(xs: &[i64]) -> PyResult<i64> {
    let mut acc: i64 = 1;
    for &x in xs {
        if x == 0 {
            return Ok(0);
        }
        let g = gcd2(acc, x);
        acc = (acc / g).checked_mul(x.abs()).ok_or_else(Exc::overflow)?;
    }
    Ok(acc)
}
pub fn factorial(n: i64) -> PyResult<i64> {
    if n < 0 {
        return Err(Exc::new("ValueError", "factorial() not defined for negative values"));
    }
    let mut r: i64 = 1;
    for k in 2..=n {
        r = r.checked_mul(k).ok_or_else(Exc::overflow)?;
    }
    Ok(r)
}
pub fn comb(n: i64, k: i64) -> PyResult<i64> {
    if n < 0 || k < 0 {
        return Err(Exc::new("ValueError", if n < 0 { "n must be a non-negative integer" } else { "k must be a non-negative integer" }));
    }
    if k > n {
        return Ok(0);
    }
    let k = k.min(n - k);
    let mut r: i128 = 1;
    for i in 0..k {
        r = r * (n - i) as i128 / (i + 1) as i128;
        if r > i64::MAX as i128 {
            return Err(Exc::overflow());
        }
    }
    Ok(r as i64)
}
pub fn perm(n: i64, k: i64) -> PyResult<i64> {
    if n < 0 || k < 0 {
        return Err(Exc::new("ValueError", if n < 0 { "n must be a non-negative integer" } else { "k must be a non-negative integer" }));
    }
    if k > n {
        return Ok(0);
    }
    let mut r: i64 = 1;
    for i in 0..k {
        r = r.checked_mul(n - i).ok_or_else(Exc::overflow)?;
    }
    Ok(r)
}
pub fn isqrt(n: i64) -> PyResult<i64> {
    if n < 0 {
        return Err(Exc::new("ValueError", "isqrt() argument must be nonnegative"));
    }
    let mut r = (n as f64).sqrt() as i64;
    while r * r > n {
        r -= 1;
    }
    while (r + 1).checked_mul(r + 1).is_some_and(|s| s <= n) {
        r += 1;
    }
    Ok(r)
}
