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

use crate::coll::{slice_positions, List};
use crate::Exc;

fn s(v: impl Into<String>) -> Str {
    Str::from(v.into())
}

thread_local! {
    /// The 256 Latin-1 one-character strings, shared (CPython caches them too):
    /// iterating or indexing a string does not allocate per character.
    static LATIN1: Vec<Str> = (0..256u32).map(|i| Str::from(char::from_u32(i).unwrap().to_string())).collect();
}

/// A one-character string.
#[inline]
pub fn char_str(c: char) -> Str {
    let u = c as u32;
    if u < 256 {
        LATIN1.with(|t| t[u as usize].clone())
    } else {
        Str::from(c.to_string())
    }
}

/// `s[i]` (code point indexing; O(1) for ASCII strings).
pub fn index(a: &Str, i: i64) -> PyResult<Str> {
    let err = || Exc::new("IndexError", "string index out of range");
    if a.is_ascii() {
        let n = a.len() as i64;
        let j = if i < 0 { i + n } else { i };
        if !(0..n).contains(&j) {
            return Err(err());
        }
        return Ok(char_str(a.as_bytes()[j as usize] as char));
    }
    let n = a.chars().count() as i64;
    let j = if i < 0 { i + n } else { i };
    if !(0..n).contains(&j) {
        return Err(err());
    }
    Ok(char_str(a.chars().nth(j as usize).unwrap()))
}

/// `s[a:b:c]`
pub fn slice(a: &Str, start: Option<i64>, stop: Option<i64>, step: Option<i64>) -> PyResult<Str> {
    if a.is_ascii() {
        if step.unwrap_or(1) == 1 {
            let (st, en, _) = crate::coll::slice_indices(start, stop, None, a.len())?;
            return Ok(if en > st { s(&a[st as usize..en as usize]) } else { s("") });
        }
        let b = a.as_bytes();
        let pos = slice_positions(start, stop, step, a.len())?;
        return Ok(s(pos.into_iter().map(|p| b[p] as char).collect::<String>()));
    }
    let cs: Vec<char> = a.chars().collect();
    let pos = slice_positions(start, stop, step, cs.len())?;
    Ok(s(pos.into_iter().map(|p| cs[p]).collect::<String>()))
}

pub fn upper(a: &Str) -> Str {
    s(a.to_uppercase())
}

pub fn lower(a: &Str) -> Str {
    s(a.to_lowercase())
}

pub fn casefold(a: &Str) -> Str {
    s(a.to_lowercase())
}

pub fn swapcase(a: &Str) -> Str {
    s(a.chars().flat_map(|c| if c.is_uppercase() { c.to_lowercase().collect::<Vec<_>>() } else { c.to_uppercase().collect() }).collect::<String>())
}

/// Python's whitespace (str.isspace / split() / strip()).
fn is_py_space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\x1c'..='\x1f')
}

pub fn strip(a: &Str, chars: Option<&Str>) -> Str {
    match chars {
        None => s(a.trim_matches(is_py_space)),
        Some(cs) => s(a.trim_matches(|c| cs.contains(c))),
    }
}

pub fn lstrip(a: &Str, chars: Option<&Str>) -> Str {
    match chars {
        None => s(a.trim_start_matches(is_py_space)),
        Some(cs) => s(a.trim_start_matches(|c| cs.contains(c))),
    }
}

pub fn rstrip(a: &Str, chars: Option<&Str>) -> Str {
    match chars {
        None => s(a.trim_end_matches(is_py_space)),
        Some(cs) => s(a.trim_end_matches(|c| cs.contains(c))),
    }
}

pub fn removeprefix(a: &Str, p: &Str) -> Str {
    s(a.strip_prefix(&**p).unwrap_or(a))
}

pub fn removesuffix(a: &Str, p: &Str) -> Str {
    s(a.strip_suffix(&**p).unwrap_or(a))
}

/// `s.split(sep=None, maxsplit=-1)`
pub fn split(a: &Str, sep: Option<&Str>, maxsplit: i64) -> PyResult<List<Str>> {
    let mut out = Vec::new();
    match sep {
        None => {
            let mut rest: &str = a.trim_start_matches(is_py_space);
            let mut n = 0;
            while !rest.is_empty() {
                if maxsplit >= 0 && n >= maxsplit {
                    out.push(s(rest));
                    break;
                }
                match rest.find(is_py_space) {
                    Some(i) => {
                        out.push(s(&rest[..i]));
                        rest = rest[i..].trim_start_matches(is_py_space);
                        n += 1;
                    }
                    None => {
                        out.push(s(rest));
                        break;
                    }
                }
            }
        }
        Some(sep) => {
            if sep.is_empty() {
                return Err(Exc::new("ValueError", "empty separator"));
            }
            if maxsplit < 0 {
                out.extend(a.split(&**sep).map(s));
            } else {
                out.extend(a.splitn(maxsplit as usize + 1, &**sep).map(s));
            }
        }
    }
    Ok(List::from_vec(out))
}

/// `s.rsplit(sep=None, maxsplit=-1)`
pub fn rsplit(a: &Str, sep: Option<&Str>, maxsplit: i64) -> PyResult<List<Str>> {
    if maxsplit < 0 {
        return split(a, sep, maxsplit);
    }
    let mut out: Vec<Str> = Vec::new();
    match sep {
        None => {
            let mut rest: &str = a.trim_end_matches(is_py_space);
            let mut n = 0;
            while !rest.is_empty() {
                if n >= maxsplit {
                    out.push(s(rest));
                    break;
                }
                match rest.rfind(is_py_space) {
                    Some(i) => {
                        let w = rest[i..].chars().next().unwrap().len_utf8();
                        out.push(s(&rest[i + w..]));
                        rest = rest[..i].trim_end_matches(is_py_space);
                        n += 1;
                    }
                    None => {
                        out.push(s(rest));
                        break;
                    }
                }
            }
        }
        Some(sep) => {
            if sep.is_empty() {
                return Err(Exc::new("ValueError", "empty separator"));
            }
            out.extend(a.rsplitn(maxsplit as usize + 1, &**sep).map(s));
        }
    }
    out.reverse();
    Ok(List::from_vec(out))
}

/// `s.splitlines(keepends=False)`
pub fn splitlines(a: &Str, keepends: bool) -> List<Str> {
    let mut out = Vec::new();
    let mut start = 0;
    let b: Vec<(usize, char)> = a.char_indices().collect();
    let mut i = 0;
    while i < b.len() {
        let (pos, c) = b[i];
        let brk = matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if brk {
            let mut end = pos + c.len_utf8();
            if c == '\r' && i + 1 < b.len() && b[i + 1].1 == '\n' {
                end += 1;
                i += 1;
            }
            out.push(s(if keepends { &a[start..end] } else { &a[start..pos] }));
            start = end;
        }
        i += 1;
    }
    if start < a.len() {
        out.push(s(&a[start..]));
    }
    List::from_vec(out)
}

/// `sep.join(items)`
pub fn join(sep: &Str, items: &[Str]) -> Str {
    let mut out = String::new();
    for (i, x) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(sep);
        }
        out.push_str(x);
    }
    s(out)
}

pub fn replace(a: &Str, old: &Str, new: &Str, count: i64) -> Str {
    if count < 0 {
        return s(a.replace(&**old, new));
    }
    s(a.replacen(&**old, new, count as usize))
}

pub fn startswith(a: &Str, p: &Str) -> bool {
    a.starts_with(&**p)
}

pub fn endswith(a: &Str, p: &Str) -> bool {
    a.ends_with(&**p)
}

fn char_pos(a: &str, byte: usize) -> i64 {
    if a.is_ascii() {
        byte as i64
    } else {
        a[..byte].chars().count() as i64
    }
}

pub fn find(a: &Str, sub: &Str) -> i64 {
    a.find(&**sub).map(|b| char_pos(a, b)).unwrap_or(-1)
}

pub fn rfind(a: &Str, sub: &Str) -> i64 {
    a.rfind(&**sub).map(|b| char_pos(a, b)).unwrap_or(-1)
}

pub fn index_of(a: &Str, sub: &Str) -> PyResult<i64> {
    match find(a, sub) {
        -1 => Err(Exc::new("ValueError", "substring not found")),
        i => Ok(i),
    }
}

pub fn rindex_of(a: &Str, sub: &Str) -> PyResult<i64> {
    match rfind(a, sub) {
        -1 => Err(Exc::new("ValueError", "substring not found")),
        i => Ok(i),
    }
}

pub fn count(a: &Str, sub: &Str) -> i64 {
    if sub.is_empty() {
        return a.chars().count() as i64 + 1;
    }
    a.matches(&**sub).count() as i64
}

pub fn isdigit(a: &Str) -> bool {
    !a.is_empty() && a.chars().all(|c| c.is_ascii_digit() || c.is_numeric() && c.to_digit(10).is_some())
}

pub fn isnumeric(a: &Str) -> bool {
    !a.is_empty() && a.chars().all(|c| c.is_numeric())
}

pub fn isdecimal(a: &Str) -> bool {
    !a.is_empty() && a.chars().all(|c| c.is_ascii_digit())
}

pub fn isalpha(a: &Str) -> bool {
    !a.is_empty() && a.chars().all(|c| c.is_alphabetic())
}

pub fn isalnum(a: &Str) -> bool {
    !a.is_empty() && a.chars().all(|c| c.is_alphanumeric())
}

pub fn isspace(a: &Str) -> bool {
    !a.is_empty() && a.chars().all(is_py_space)
}

pub fn isupper(a: &Str) -> bool {
    a.chars().any(|c| c.is_uppercase()) && !a.chars().any(|c| c.is_lowercase())
}

pub fn islower(a: &Str) -> bool {
    a.chars().any(|c| c.is_lowercase()) && !a.chars().any(|c| c.is_uppercase())
}

pub fn isidentifier(a: &Str) -> bool {
    let mut cs = a.chars();
    match cs.next() {
        Some(c) if c == '_' || c.is_alphabetic() => cs.all(|c| c == '_' || c.is_alphanumeric()),
        _ => false,
    }
}

pub fn istitle(a: &Str) -> bool {
    let mut prev_cased = false;
    let mut any = false;
    for c in a.chars() {
        if c.is_uppercase() {
            if prev_cased {
                return false;
            }
            prev_cased = true;
            any = true;
        } else if c.is_lowercase() {
            if !prev_cased {
                return false;
            }
            prev_cased = true;
            any = true;
        } else {
            prev_cased = false;
        }
    }
    any
}

pub fn title(a: &Str) -> Str {
    let mut out = String::with_capacity(a.len());
    let mut prev_cased = false;
    for c in a.chars() {
        if prev_cased {
            out.extend(c.to_lowercase());
        } else {
            out.extend(c.to_uppercase());
        }
        prev_cased = c.is_alphabetic();
    }
    s(out)
}

pub fn capitalize(a: &Str) -> Str {
    let mut cs = a.chars();
    match cs.next() {
        None => s(""),
        Some(f) => s(f.to_uppercase().collect::<String>() + &cs.as_str().to_lowercase()),
    }
}

fn fill_char(f: Option<&Str>) -> PyResult<char> {
    match f {
        None => Ok(' '),
        Some(f) => {
            let mut cs = f.chars();
            match (cs.next(), cs.next()) {
                (Some(c), None) => Ok(c),
                _ => Err(Exc::new("TypeError", "The fill character must be exactly one character long")),
            }
        }
    }
}

pub fn center(a: &Str, width: i64, fill: Option<&Str>) -> PyResult<Str> {
    let f = fill_char(fill)?;
    let n = a.chars().count() as i64;
    if width <= n {
        return Ok(a.clone());
    }
    let total = width - n;
    // CPython: left = total/2 + (total & width & 1)
    let left = total / 2 + (total & width & 1);
    let right = total - left;
    Ok(s(std::iter::repeat_n(f, left as usize).chain(a.chars()).chain(std::iter::repeat_n(f, right as usize)).collect::<String>()))
}

pub fn ljust(a: &Str, width: i64, fill: Option<&Str>) -> PyResult<Str> {
    let f = fill_char(fill)?;
    let n = a.chars().count() as i64;
    Ok(if width <= n { a.clone() } else { s(a.to_string() + &f.to_string().repeat((width - n) as usize)) })
}

pub fn rjust(a: &Str, width: i64, fill: Option<&Str>) -> PyResult<Str> {
    let f = fill_char(fill)?;
    let n = a.chars().count() as i64;
    Ok(if width <= n { a.clone() } else { s(f.to_string().repeat((width - n) as usize) + a) })
}

pub fn zfill(a: &Str, width: i64) -> Str {
    let n = a.chars().count() as i64;
    if width <= n {
        return a.clone();
    }
    let pad = "0".repeat((width - n) as usize);
    match a.chars().next() {
        Some(c @ ('+' | '-')) => s(format!("{c}{pad}{}", &a[1..])),
        _ => s(pad + a),
    }
}

/// `s.partition(sep)` / `s.rpartition(sep)`
pub fn partition(a: &Str, sep: &Str, from_right: bool) -> PyResult<(Str, Str, Str)> {
    if sep.is_empty() {
        return Err(Exc::new("ValueError", "empty separator"));
    }
    let found = if from_right { a.rfind(&**sep) } else { a.find(&**sep) };
    Ok(match found {
        Some(i) => (s(&a[..i]), sep.clone(), s(&a[i + sep.len()..])),
        None if from_right => (s(""), s(""), a.clone()),
        None => (a.clone(), s(""), s("")),
    })
}

pub fn expandtabs(a: &Str, tabsize: i64) -> Str {
    let mut out = String::new();
    let mut col = 0i64;
    for c in a.chars() {
        match c {
            '\t' => {
                if tabsize > 0 {
                    let n = tabsize - col % tabsize;
                    out.push_str(&" ".repeat(n as usize));
                    col += n;
                }
            }
            '\n' | '\r' => {
                out.push(c);
                col = 0;
            }
            c => {
                out.push(c);
                col += 1;
            }
        }
    }
    s(out)
}

/// `ord(c)`
pub fn ord(a: &Str) -> PyResult<i64> {
    let mut cs = a.chars();
    match (cs.next(), cs.next()) {
        (Some(c), None) => Ok(c as i64),
        _ => Err(Exc::new("TypeError", format!("ord() expected a character, but string of length {} found", a.chars().count()))),
    }
}

/// `chr(i)`
pub fn chr(i: i64) -> PyResult<Str> {
    u32::try_from(i)
        .ok()
        .and_then(char::from_u32)
        .map(char_str)
        .ok_or_else(|| Exc::new("ValueError", "chr() arg not in range(0x110000)"))
}

/// `a < b` etc. on strings is byte order (= code point order for UTF-8).
pub fn cmp(a: &Str, b: &Str) -> std::cmp::Ordering {
    a.as_bytes().cmp(b.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conv::PyFmt;

    #[test]
    fn methods() {
        let t = Str::from("  a  b c ");
        assert_eq!(&*split(&t, None, -1).unwrap().py_repr(), "['a', 'b', 'c']");
        assert_eq!(&*split(&t, None, 1).unwrap().py_repr(), "['a', 'b c ']");
        assert_eq!(&*rsplit(&t, None, 1).unwrap().py_repr(), "['  a  b', 'c']");
        assert_eq!(&*split(&Str::from("a,b,,c"), Some(&Str::from(",")), -1).unwrap().py_repr(), "['a', 'b', '', 'c']");
        assert_eq!(&*slice(&Str::from("hello"), None, None, Some(-1)).unwrap(), "olleh");
        assert_eq!(&*slice(&Str::from("héllo"), Some(1), Some(3), None).unwrap(), "él");
        assert_eq!(&*index(&Str::from("héllo"), -4).unwrap(), "é");
        assert_eq!(&*center(&Str::from("ab"), 7, Some(&Str::from("*"))).unwrap(), "***ab**");
        assert_eq!(&*title(&Str::from("hello wORLD-x")), "Hello World-X");
        assert_eq!(find(&Str::from("héllo"), &Str::from("l")), 2);
        assert_eq!(&*zfill(&Str::from("-42"), 5), "-0042");
        assert_eq!(&*splitlines(&Str::from("a\r\nb\nc"), false).py_repr(), "['a', 'b', 'c']");
    }
}
