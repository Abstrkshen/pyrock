//! Builtin conversions: `int()`, `float()`, `str()`.

use crate::exc::{Exc, PyResult};
use crate::{float, string, Str};

pub fn int_from_float(f: f64) -> PyResult<i64> {
    if f.is_nan() {
        return Err(Exc::new("ValueError", "cannot convert float NaN to integer"));
    }
    if f.is_infinite() {
        return Err(Exc::new("OverflowError", "cannot convert float infinity to integer"));
    }
    let t = f.trunc();
    if !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&t) {
        return Err(Exc::overflow());
    }
    Ok(t as i64)
}

pub fn int_from_str(s: &Str) -> PyResult<i64> {
    let err = || Exc::new("ValueError", format!("invalid literal for int() with base 10: {}", string::repr(s)));
    let t = s.trim();
    let (neg, body) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    if body.is_empty() || body.starts_with('_') || body.ends_with('_') || body.contains("__") {
        return Err(err());
    }
    let mut v: i128 = 0;
    for c in body.chars() {
        if c == '_' {
            continue;
        }
        let d = c.to_digit(10).ok_or_else(err)? as i128;
        v = v * 10 + d;
        if v > (i64::MAX as i128) + 1 {
            return Err(Exc::overflow());
        }
    }
    let v = if neg { -v } else { v };
    i64::try_from(v).map_err(|_| Exc::overflow())
}

pub fn float_from_str(s: &Str) -> PyResult<f64> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    let body = lower.trim_start_matches(['+', '-']);
    let ok_word = matches!(body, "inf" | "infinity" | "nan");
    let ok_num = !body.is_empty()
        && body.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | '+' | '-' | '_'))
        && !body.starts_with('_')
        && !body.contains("__");
    if ok_word || ok_num {
        if let Ok(v) = lower.replace('_', "").parse::<f64>() {
            return Ok(v);
        }
    }
    Err(Exc::new("ValueError", format!("could not convert string to float: {}", string::repr(s))))
}

pub fn str_int(v: i64) -> Str {
    Str::from(v.to_string())
}

pub fn str_float(v: f64) -> Str {
    Str::from(float::repr(v))
}

pub fn str_bool(v: bool) -> Str {
    Str::from(if v { "True" } else { "False" })
}

pub fn str_none() -> Str {
    Str::from("None")
}
