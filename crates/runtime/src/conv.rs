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

/// Python `str()` / `repr()` for every native value type. Collections use
/// `py_repr` for their elements, as CPython does.
pub trait PyFmt {
    fn py_repr(&self) -> Str;
    fn py_str(&self) -> Str {
        self.py_repr()
    }
}

/// `str(x)`
pub fn str_of<T: PyFmt + ?Sized>(v: &T) -> Str {
    v.py_str()
}

/// `repr(x)`
pub fn repr_of<T: PyFmt + ?Sized>(v: &T) -> Str {
    v.py_repr()
}

impl PyFmt for i64 {
    fn py_repr(&self) -> Str {
        str_int(*self)
    }
}

impl PyFmt for f64 {
    fn py_repr(&self) -> Str {
        str_float(*self)
    }
}

impl PyFmt for bool {
    fn py_repr(&self) -> Str {
        str_bool(*self)
    }
}

impl PyFmt for () {
    fn py_repr(&self) -> Str {
        str_none()
    }
}

impl PyFmt for Str {
    fn py_repr(&self) -> Str {
        Str::from(string::repr(self))
    }
    fn py_str(&self) -> Str {
        self.clone()
    }
}

impl<T: PyFmt> PyFmt for Option<T> {
    fn py_repr(&self) -> Str {
        match self {
            Some(v) => v.py_repr(),
            None => str_none(),
        }
    }
    fn py_str(&self) -> Str {
        match self {
            Some(v) => v.py_str(),
            None => str_none(),
        }
    }
}

impl PyFmt for Exc {
    fn py_repr(&self) -> Str {
        #[cfg(feature = "python")]
        if self.origin.is_some() {
            if let Ok(o) = crate::py::exc_obj(self) {
                return o.py_repr();
            }
        }
        // KeyError keeps its argument's repr as the message already.
        if self.msg.is_empty() {
            Str::from(format!("{}()", self.name()))
        } else if self.name() == "KeyError" {
            Str::from(format!("{}({})", self.name(), self.msg))
        } else {
            Str::from(format!("{}({})", self.name(), string::repr(&self.msg)))
        }
    }
    fn py_str(&self) -> Str {
        Str::from(self.msg.as_str())
    }
}
