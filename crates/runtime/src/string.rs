//! Python `str` operations on immutable, reference-counted UTF-8 strings.
//! Rust `str` ordering is byte-wise UTF-8, which equals code-point order.

use crate::exc::PyResult;
use crate::Str;

pub fn concat(a: &Str, b: &Str) -> Str {
    let mut s = String::with_capacity(a.len() + b.len());
    s.push_str(a);
    s.push_str(b);
    Str::from(s)
}

pub fn repeat(a: &Str, n: i64) -> PyResult<Str> {
    if n <= 0 || a.is_empty() {
        return Ok(Str::from(""));
    }
    let n = usize::try_from(n).map_err(|_| crate::Exc::new("MemoryError", ""))?;
    if a.len().checked_mul(n).is_none() {
        return Err(crate::Exc::new("OverflowError", "repeated string is too long"));
    }
    Ok(Str::from(a.repeat(n)))
}

/// `len(s)` counts code points.
pub fn len(a: &Str) -> i64 {
    a.chars().count() as i64
}

/// Python `repr(str)`.
pub fn repr(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(q);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

/// `item in container` for two strings (substring test).
pub fn contains(item: &Str, container: &Str) -> bool {
    container.contains(&**item)
}
