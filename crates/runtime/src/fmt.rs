//! Python string formatting: the format-spec mini-language (`format()`,
//! f-string `{x:spec}`, `str.format`) and printf-style `%` formatting.

use crate::conv::PyFmt;
use crate::exc::{Exc, PyResult};
use crate::{float, Str};

/// A value being formatted.
#[derive(Clone)]
pub enum Arg {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Str),
    /// Any other value: its `str()` and `repr()`, and its type name for errors.
    Other { s: Str, r: Str, ty: &'static str },
    #[cfg(feature = "python")]
    Py(crate::py::Obj),
}

impl Arg {
    fn type_name(&self) -> &'static str {
        match self {
            Arg::Int(_) => "int",
            Arg::Float(_) => "float",
            Arg::Bool(_) => "bool",
            Arg::Str(_) => "str",
            Arg::Other { ty, .. } => ty,
            #[cfg(feature = "python")]
            Arg::Py(_) => "object",
        }
    }

    fn str_(&self) -> Str {
        match self {
            Arg::Int(v) => v.py_str(),
            Arg::Float(v) => v.py_str(),
            Arg::Bool(v) => v.py_str(),
            Arg::Str(s) => s.clone(),
            Arg::Other { s, .. } => s.clone(),
            #[cfg(feature = "python")]
            Arg::Py(o) => o.py_str(),
        }
    }

    fn repr(&self) -> Str {
        match self {
            Arg::Int(v) => v.py_repr(),
            Arg::Float(v) => v.py_repr(),
            Arg::Bool(v) => v.py_repr(),
            Arg::Str(s) => s.py_repr(),
            Arg::Other { r, .. } => r.clone(),
            #[cfg(feature = "python")]
            Arg::Py(o) => o.py_repr(),
        }
    }
}

/// Build an `Arg` from any native value (non-scalar values format through `str()`).
pub fn other<T: PyFmt + ?Sized>(v: &T, ty: &'static str) -> Arg {
    Arg::Other { s: v.py_str(), r: v.py_repr(), ty }
}

#[derive(Default, Debug)]
struct Spec {
    fill: Option<char>,
    align: Option<char>,
    sign: Option<char>,
    alt: bool,
    zero: bool,
    width: usize,
    grouping: Option<char>,
    precision: Option<usize>,
    ty: Option<char>,
}

fn parse_spec(spec: &str) -> PyResult<Spec> {
    let cs: Vec<char> = spec.chars().collect();
    let mut i = 0;
    let mut s = Spec::default();
    let is_align = |c: char| matches!(c, '<' | '>' | '^' | '=');
    if cs.len() >= 2 && is_align(cs[1]) {
        s.fill = Some(cs[0]);
        s.align = Some(cs[1]);
        i = 2;
    } else if !cs.is_empty() && is_align(cs[0]) {
        s.align = Some(cs[0]);
        i = 1;
    }
    if i < cs.len() && matches!(cs[i], '+' | '-' | ' ') {
        s.sign = Some(cs[i]);
        i += 1;
    }
    if i < cs.len() && cs[i] == 'z' {
        i += 1;
    }
    if i < cs.len() && cs[i] == '#' {
        s.alt = true;
        i += 1;
    }
    if i < cs.len() && cs[i] == '0' {
        s.zero = true;
        i += 1;
    }
    let start = i;
    while i < cs.len() && cs[i].is_ascii_digit() {
        i += 1;
    }
    if i > start {
        s.width = cs[start..i].iter().collect::<String>().parse().unwrap_or(0);
    }
    if i < cs.len() && matches!(cs[i], ',' | '_') {
        s.grouping = Some(cs[i]);
        i += 1;
    }
    if i < cs.len() && cs[i] == '.' {
        i += 1;
        let ps = i;
        while i < cs.len() && cs[i].is_ascii_digit() {
            i += 1;
        }
        if i == ps {
            return Err(Exc::new("ValueError", "Format specifier missing precision"));
        }
        s.precision = Some(cs[ps..i].iter().collect::<String>().parse().unwrap_or(0));
    }
    if i < cs.len() {
        s.ty = Some(cs[i]);
        i += 1;
    }
    if i != cs.len() {
        return Err(Exc::new("ValueError", "Invalid format specifier"));
    }
    Ok(s)
}

fn group(digits: &str, sep: char, every: usize) -> String {
    let n = digits.len();
    let mut out = String::with_capacity(n + n / every);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (n - i) % every == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// Pad `body` (with `sign_prefix` kept in front for `=` alignment).
fn pad(s: &Spec, sign_prefix: &str, body: &str, numeric: bool) -> String {
    let len = sign_prefix.chars().count() + body.chars().count();
    if s.width <= len {
        return format!("{sign_prefix}{body}");
    }
    let n = s.width - len;
    let (fill, align) = if s.zero && s.align.is_none() && numeric {
        ('0', '=')
    } else {
        (s.fill.unwrap_or(' '), s.align.unwrap_or(if numeric { '>' } else { '<' }))
    };
    let f = |k: usize| std::iter::repeat_n(fill, k).collect::<String>();
    match align {
        '<' => format!("{sign_prefix}{body}{}", f(n)),
        '>' => format!("{}{sign_prefix}{body}", f(n)),
        '^' => format!("{}{sign_prefix}{body}{}", f(n / 2), f(n - n / 2)),
        _ => format!("{sign_prefix}{}{body}", f(n)),
    }
}

fn sign_of(neg: bool, s: &Spec) -> &'static str {
    if neg {
        "-"
    } else {
        match s.sign {
            Some('+') => "+",
            Some(' ') => " ",
            _ => "",
        }
    }
}

fn fmt_int(v: i64, s: &Spec) -> PyResult<String> {
    let ty = s.ty.unwrap_or('d');
    if matches!(ty, 'e' | 'E' | 'f' | 'F' | 'g' | 'G' | '%') {
        return fmt_float(v as f64, s);
    }
    if s.precision.is_some() {
        return Err(Exc::new("ValueError", "Precision not allowed in integer format specifier"));
    }
    let neg = v < 0;
    let a = v.unsigned_abs();
    let (digits, prefix, every) = match ty {
        'd' | 'n' => (a.to_string(), "", 3),
        'x' => (format!("{a:x}"), "0x", 4),
        'X' => (format!("{a:X}"), "0X", 4),
        'o' => (format!("{a:o}"), "0o", 4),
        'b' => (format!("{a:b}"), "0b", 4),
        'c' => {
            let c = u32::try_from(v).ok().and_then(char::from_u32).ok_or_else(|| Exc::new("OverflowError", "%c arg not in range(0x110000)"))?;
            return Ok(pad(s, "", &c.to_string(), false));
        }
        _ => return Err(Exc::new("ValueError", format!("Unknown format code '{ty}' for object of type 'int'"))),
    };
    let digits = match s.grouping {
        Some(g) => group(&digits, g, every),
        None => digits,
    };
    let sign = format!("{}{}", sign_of(neg, s), if s.alt { prefix } else { "" });
    Ok(pad(s, &sign, &digits, true))
}

/// CPython's `repr`-style exponent: `e+05`.
fn fix_exp(m: &str, upper: bool) -> String {
    let (mant, exp) = m.split_once('e').unwrap_or((m, "0"));
    let e: i32 = exp.parse().unwrap_or(0);
    let ech = if upper { 'E' } else { 'e' };
    format!("{mant}{ech}{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
}

/// `%g` with `p` significant digits (CPython's rules).
fn general(a: f64, p: usize, alt: bool, upper: bool, force_point: bool) -> String {
    let p = p.max(1);
    if a == 0.0 {
        let mut s = String::from("0");
        if alt {
            s.push('.');
            s.push_str(&"0".repeat(p - 1));
        } else if force_point {
            s.push_str(".0");
        }
        return s;
    }
    // Exponent after rounding to p significant digits.
    let e_repr = format!("{:.*e}", p - 1, a);
    let exp: i32 = e_repr.split_once('e').map(|(_, e)| e.parse().unwrap_or(0)).unwrap_or(0);
    let strip = |s: String| -> String {
        if alt || !s.contains('.') {
            return s;
        }
        let t = s.trim_end_matches('0').trim_end_matches('.').to_string();
        t
    };
    if -4 <= exp && (exp as i64) < p as i64 {
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        let mut s = strip(format!("{:.*}", decimals, a));
        if force_point && !s.contains('.') {
            s.push_str(".0");
        }
        s
    } else {
        let (mant, _) = e_repr.split_once('e').unwrap();
        let mant = strip(mant.to_string());
        fix_exp(&format!("{mant}e{exp}"), upper)
    }
}

fn fmt_float(v: f64, s: &Spec) -> PyResult<String> {
    let neg = v.is_sign_negative() && !(v.is_nan());
    let a = v.abs();
    let ty = s.ty;
    let upper = matches!(ty, Some('E' | 'F' | 'G'));
    let body = if a.is_nan() || a.is_infinite() {
        let w = if a.is_nan() { "nan" } else { "inf" };
        if upper { w.to_uppercase() } else { w.to_string() }
    } else {
        match ty {
            Some('f' | 'F') => format!("{:.*}", s.precision.unwrap_or(6), a),
            Some('e' | 'E') => fix_exp(&format!("{:.*e}", s.precision.unwrap_or(6), a), upper),
            Some('%') => format!("{:.*}%", s.precision.unwrap_or(6), a * 100.0),
            Some('g' | 'G') => general(a, s.precision.unwrap_or(6), s.alt, upper, false),
            None | Some('n') => match s.precision {
                None => float::repr(a),
                Some(p) => general(a, p, s.alt, false, true),
            },
            Some(t) => return Err(Exc::new("ValueError", format!("Unknown format code '{t}' for object of type 'float'"))),
        }
    };
    let body = match s.grouping {
        Some(g) if !body.contains(['e', 'E', 'n', 'N']) => {
            let (int, rest) = match body.find('.') {
                Some(i) => (&body[..i], &body[i..]),
                None => match body.find('%') {
                    Some(i) => (&body[..i], &body[i..]),
                    None => (&body[..], ""),
                },
            };
            format!("{}{rest}", group(int, g, 3))
        }
        _ => body,
    };
    let body = if s.alt && !body.contains('.') && matches!(ty, Some('f' | 'F' | 'e' | 'E')) {
        match body.find(['e', 'E']) {
            Some(i) => format!("{}.{}", &body[..i], &body[i..]),
            None => format!("{body}."),
        }
    } else {
        body
    };
    Ok(pad(s, sign_of(neg, s), &body, true))
}

fn fmt_str(v: &str, s: &Spec, tyname: &str) -> PyResult<String> {
    match s.ty {
        None | Some('s') => {}
        Some(t) => return Err(Exc::new("ValueError", format!("Unknown format code '{t}' for object of type '{tyname}'"))),
    }
    if s.sign.is_some() {
        return Err(Exc::new("ValueError", "Sign not allowed in string format specifier"));
    }
    let body: String = match s.precision {
        Some(p) => v.chars().take(p).collect(),
        None => v.to_string(),
    };
    Ok(pad(s, "", &body, false))
}

/// `format(value, spec)`
pub fn format(v: &Arg, spec: &str) -> PyResult<Str> {
    #[cfg(feature = "python")]
    if let Arg::Py(o) = v {
        return crate::py::format(o, spec);
    }
    if spec.is_empty() {
        return Ok(v.str_());
    }
    let s = parse_spec(spec)?;
    let out = match v {
        Arg::Int(i) => fmt_int(*i, &s)?,
        Arg::Bool(b) if s.ty.is_some() && s.ty != Some('s') => fmt_int(*b as i64, &s)?,
        Arg::Bool(b) => fmt_str(if *b { "True" } else { "False" }, &s, "bool")?,
        Arg::Float(f) => fmt_float(*f, &s)?,
        Arg::Str(x) => fmt_str(x, &s, "str")?,
        Arg::Other { s: st, ty, .. } => {
            return Err(Exc::new("TypeError", format!("unsupported format string passed to {ty}.__format__")).map_noop(st));
        }
        #[cfg(feature = "python")]
        Arg::Py(_) => unreachable!(),
    };
    Ok(Str::from(out))
}

trait MapNoop {
    fn map_noop(self, _: &Str) -> Self;
}
impl MapNoop for Exc {
    fn map_noop(self, _: &Str) -> Self {
        self
    }
}

/// `fmt % args` (printf-style). `args` is the tuple (or the single value).
pub fn percent(fmt: &str, args: &[Arg]) -> PyResult<Str> {
    let mut out = String::with_capacity(fmt.len() + 16);
    let cs: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    let mut k = 0;
    let next = |k: &mut usize| -> PyResult<Arg> {
        let a = args.get(*k).cloned().ok_or_else(|| Exc::new("TypeError", "not enough arguments for format string"))?;
        *k += 1;
        Ok(a)
    };
    while i < cs.len() {
        let c = cs[i];
        if c != '%' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        if i >= cs.len() {
            return Err(Exc::new("ValueError", "incomplete format"));
        }
        if cs[i] == '%' {
            out.push('%');
            i += 1;
            continue;
        }
        // flags
        let mut spec = Spec::default();
        let mut left = false;
        while i < cs.len() && matches!(cs[i], '-' | '+' | ' ' | '#' | '0') {
            match cs[i] {
                '-' => left = true,
                '+' => spec.sign = Some('+'),
                ' ' => {
                    if spec.sign.is_none() {
                        spec.sign = Some(' ')
                    }
                }
                '#' => spec.alt = true,
                _ => spec.zero = true,
            }
            i += 1;
        }
        if i < cs.len() && cs[i] == '*' {
            spec.width = match next(&mut k)? {
                Arg::Int(w) => w.max(0) as usize,
                _ => return Err(Exc::new("TypeError", "* wants int")),
            };
            i += 1;
        } else {
            let st = i;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
            if i > st {
                spec.width = cs[st..i].iter().collect::<String>().parse().unwrap_or(0);
            }
        }
        if i < cs.len() && cs[i] == '.' {
            i += 1;
            let st = i;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
            spec.precision = Some(cs[st..i].iter().collect::<String>().parse().unwrap_or(0));
        }
        if i >= cs.len() {
            return Err(Exc::new("ValueError", "incomplete format"));
        }
        let conv = cs[i];
        i += 1;
        if left {
            spec.align = Some('<');
            spec.zero = false;
        }
        let a = next(&mut k)?;
        let piece = match conv {
            's' | 'r' | 'a' => {
                let text = if conv == 's' { a.str_() } else { a.repr() };
                let mut ss = Spec { align: Some(if left { '<' } else { '>' }), width: spec.width, precision: spec.precision, ..Spec::default() };
                ss.fill = Some(' ');
                fmt_str(&text, &ss, "str")?
            }
            'd' | 'i' | 'u' => {
                let v = match a {
                    Arg::Int(v) => v,
                    Arg::Bool(b) => b as i64,
                    Arg::Float(f) => crate::conv::int_from_float(f)?,
                    other => return Err(Exc::new("TypeError", format!("%{conv} format: a real number is required, not {}", other.type_name()))),
                };
                if !left {
                    spec.align = None;
                }
                let s2 = Spec { ty: Some('d'), precision: None, ..spec };
                let mut r = fmt_int(v, &s2)?;
                if let Some(p) = s2.precision {
                    let _ = p;
                }
                if left {
                    r = format!("{:<w$}", r.trim_end(), w = s2.width);
                }
                r
            }
            'x' | 'X' | 'o' => {
                let v = match a {
                    Arg::Int(v) => v,
                    Arg::Bool(b) => b as i64,
                    other => return Err(Exc::new("TypeError", format!("%{conv} format: an integer is required, not {}", other.type_name()))),
                };
                if !left {
                    spec.align = None;
                }
                let s2 = Spec { ty: Some(conv), precision: None, ..spec };
                fmt_int(v, &s2)?
            }
            'e' | 'E' | 'f' | 'F' | 'g' | 'G' => {
                let v = match a {
                    Arg::Int(v) => v as f64,
                    Arg::Bool(b) => b as i64 as f64,
                    Arg::Float(f) => f,
                    other => return Err(Exc::new("TypeError", format!("must be real number, not {}", other.type_name()))),
                };
                if !left {
                    spec.align = None;
                }
                let s2 = Spec { ty: Some(conv), precision: Some(spec.precision.unwrap_or(6)), ..spec };
                fmt_float(v, &s2)?
            }
            'c' => match a {
                Arg::Int(v) => fmt_int(v, &Spec { ty: Some('c'), ..spec })?,
                Arg::Str(s) if s.chars().count() == 1 => s.to_string(),
                _ => return Err(Exc::new("TypeError", "%c requires an int or a unicode character")),
            },
            other => return Err(Exc::new("ValueError", format!("unsupported format character '{other}' (0x{:x}) at index {}", other as u32, i - 1))),
        };
        out.push_str(&piece);
    }
    if k < args.len() {
        return Err(Exc::new("TypeError", "not all arguments converted during string formatting"));
    }
    Ok(Str::from(out))
}

/// `fmt.format(*args, **kwargs)`.
pub fn format_method(fmt: &str, args: &[Arg], kwargs: &[(&str, Arg)]) -> PyResult<Str> {
    let mut out = String::with_capacity(fmt.len() + 16);
    let cs: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    let mut auto = 0usize;
    let mut manual = false;
    while i < cs.len() {
        let c = cs[i];
        if c == '{' {
            if i + 1 < cs.len() && cs[i + 1] == '{' {
                out.push('{');
                i += 2;
                continue;
            }
            // Find the matching '}' (one level of nesting for `{x:{w}}`).
            let mut depth = 1;
            let mut j = i + 1;
            while j < cs.len() && depth > 0 {
                match cs[j] {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                if depth > 0 {
                    j += 1;
                }
            }
            if j >= cs.len() {
                return Err(Exc::new("ValueError", "Single '{' encountered in format string"));
            }
            let field: String = cs[i + 1..j].iter().collect();
            i = j + 1;
            let (head, spec) = match field.find(':') {
                Some(p) => (&field[..p], &field[p + 1..]),
                None => (&field[..], ""),
            };
            let (name, conv) = match head.find('!') {
                Some(p) => (&head[..p], Some(&head[p + 1..])),
                None => (head, None),
            };
            let arg = if name.is_empty() {
                if manual {
                    return Err(Exc::new("ValueError", "cannot switch from manual field specification to automatic field numbering"));
                }
                auto += 1;
                args.get(auto - 1).cloned().ok_or_else(|| Exc::new("IndexError", format!("Replacement index {} out of range for positional args tuple", auto - 1)))?
            } else if let Ok(n) = name.parse::<usize>() {
                if auto > 0 {
                    return Err(Exc::new("ValueError", "cannot switch from automatic field numbering to manual field specification"));
                }
                manual = true;
                args.get(n).cloned().ok_or_else(|| Exc::new("IndexError", format!("Replacement index {n} out of range for positional args tuple")))?
            } else {
                if name.contains(['.', '[']) {
                    return Err(Exc::new("NotImplementedError", "attribute/index lookups in format fields are not native"));
                }
                kwargs.iter().find(|(k, _)| *k == name).map(|(_, a)| a.clone()).ok_or_else(|| Exc::new("KeyError", format!("'{name}'")))?
            };
            let arg = match conv {
                None => arg,
                Some("s") => Arg::Str(arg.str_()),
                Some("r") => Arg::Str(arg.repr()),
                Some("a") => Arg::Str(crate::conv::ascii_of(&arg.repr())),
                Some(o) => return Err(Exc::new("ValueError", format!("Unknown conversion specifier {o}"))),
            };
            // Nested replacement fields inside the spec.
            let spec = if spec.contains('{') {
                let inner = format_method(spec, args, kwargs)?;
                inner.to_string()
            } else {
                spec.to_string()
            };
            out.push_str(&format(&arg, &spec)?);
        } else if c == '}' {
            if i + 1 < cs.len() && cs[i + 1] == '}' {
                out.push('}');
                i += 2;
                continue;
            }
            return Err(Exc::new("ValueError", "Single '}' encountered in format string"));
        } else {
            out.push(c);
            i += 1;
        }
    }
    Ok(Str::from(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: Arg, spec: &str) -> String {
        format(&v, spec).unwrap().to_string()
    }

    #[test]
    fn specs() {
        assert_eq!(f(Arg::Float(3.14159), ".2f"), "3.14");
        assert_eq!(f(Arg::Float(2.675), ".2f"), "2.67");
        assert_eq!(f(Arg::Float(1234567.891), ",.2f"), "1,234,567.89");
        assert_eq!(f(Arg::Int(1234567), ","), "1,234,567");
        assert_eq!(f(Arg::Int(255), "#x"), "0xff");
        assert_eq!(f(Arg::Int(-42), "08d"), "-0000042");
        assert_eq!(f(Arg::Str(Str::from("ab")), ">5"), "   ab");
        assert_eq!(f(Arg::Str(Str::from("ab")), "*^6"), "**ab**");
        assert_eq!(f(Arg::Float(12345.678), "e"), "1.234568e+04");
        assert_eq!(f(Arg::Float(0.0001234), "g"), "0.0001234");
        assert_eq!(f(Arg::Float(123456789.0), "g"), "1.23457e+08");
        assert_eq!(f(Arg::Float(1.0), ".3"), "1.0");
        assert_eq!(f(Arg::Float(1234.0), ".3"), "1.23e+03");
        assert_eq!(f(Arg::Float(0.25), ".1%"), "25.0%");
        assert_eq!(f(Arg::Int(5), "+d"), "+5");
        assert_eq!(f(Arg::Float(-0.0), ".1f"), "-0.0");
        assert_eq!(f(Arg::Int(42), "f"), "42.000000");
    }

    #[test]
    fn percent_and_format() {
        let r = percent("%s is %d years, %.1f%% done, %5s|%-5s|", &[Arg::Str(Str::from("Ann")), Arg::Int(30), Arg::Float(99.5), Arg::Str(Str::from("x")), Arg::Str(Str::from("y"))]).unwrap();
        assert_eq!(&*r, "Ann is 30 years, 99.5% done,     x|y    |");
        assert_eq!(&*percent("%05.1f|%x|%r", &[Arg::Float(3.14159), Arg::Int(255), Arg::Str(Str::from("q"))]).unwrap(), "003.1|ff|'q'");
        let r = format_method("{0} + {1} = {total:>4}, {0!r}", &[Arg::Str(Str::from("a")), Arg::Int(2)], &[("total", Arg::Int(3))]).unwrap();
        assert_eq!(&*r, "a + 2 =    3, 'a'");
        assert_eq!(&*format_method("{:{w}.{p}f}", &[Arg::Float(3.14159)], &[("w", Arg::Int(8)), ("p", Arg::Int(2))]).unwrap(), "    3.14");
    }
}
