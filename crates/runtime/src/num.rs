//! `int | float`: a value that is an int on some paths and a float on
//! others (`x = 0` then `x += 0.5`). Each operation dispatches on the
//! variants with exactly the int or float semantics CPython would use, so
//! `760` stays an int and prints as `760`, while `760.5` is a float.

use crate::exc::{Exc, PyResult};
use crate::{float, int, Str};

#[derive(Clone, Copy, Debug)]
pub enum Num {
    I(i64),
    F(f64),
}

impl Default for Num {
    fn default() -> Num {
        Num::I(0)
    }
}

use Num::{F, I};

impl Num {
    #[inline]
    pub fn to_f64(self) -> f64 {
        match self {
            I(i) => i as f64,
            F(f) => f,
        }
    }

    /// `int(x)`: truncates a float (errors for inf and NaN).
    pub fn to_int(self) -> PyResult<i64> {
        match self {
            I(i) => Ok(i),
            F(f) => crate::conv::int_from_float(f),
        }
    }

    /// Where an int is required (`range(x)`, `s[x]`): a float is a TypeError.
    pub fn to_index(self) -> PyResult<i64> {
        match self {
            I(i) => Ok(i),
            F(_) => Err(Exc::new("TypeError", "'float' object cannot be interpreted as an integer")),
        }
    }
}

#[inline]
pub fn add(a: Num, b: Num) -> PyResult<Num> {
    Ok(match (a, b) {
        (I(x), I(y)) => I(int::add(x, y)?),
        _ => F(a.to_f64() + b.to_f64()),
    })
}
#[inline]
pub fn sub(a: Num, b: Num) -> PyResult<Num> {
    Ok(match (a, b) {
        (I(x), I(y)) => I(int::sub(x, y)?),
        _ => F(a.to_f64() - b.to_f64()),
    })
}
#[inline]
pub fn mul(a: Num, b: Num) -> PyResult<Num> {
    Ok(match (a, b) {
        (I(x), I(y)) => I(int::mul(x, y)?),
        _ => F(a.to_f64() * b.to_f64()),
    })
}
pub fn floordiv(a: Num, b: Num) -> PyResult<Num> {
    Ok(match (a, b) {
        (I(x), I(y)) => I(int::floordiv(x, y)?),
        _ => F(float::floordiv(a.to_f64(), b.to_f64())?),
    })
}
pub fn modulo(a: Num, b: Num) -> PyResult<Num> {
    Ok(match (a, b) {
        (I(x), I(y)) => I(int::modulo(x, y)?),
        _ => F(float::modulo(a.to_f64(), b.to_f64())?),
    })
}
pub fn pow(a: Num, b: Num) -> PyResult<Num> {
    Ok(match (a, b) {
        (I(x), I(y)) if y >= 0 => I(int::pow(x, y)?),
        _ => F(float::pow(a.to_f64(), b.to_f64())?),
    })
}
pub fn truediv(a: Num, b: Num) -> PyResult<f64> {
    match (a, b) {
        (I(x), I(y)) => int::truediv(x, y),
        _ => float::truediv(a.to_f64(), b.to_f64()),
    }
}
pub fn neg(a: Num) -> PyResult<Num> {
    Ok(match a {
        I(x) => I(int::neg(x)?),
        F(f) => F(-f),
    })
}
pub fn abs(a: Num) -> PyResult<Num> {
    Ok(match a {
        I(x) => I(int::abs(x)?),
        F(f) => F(f.abs()),
    })
}
/// `round(x)`
pub fn round(a: Num) -> PyResult<i64> {
    match a {
        I(x) => Ok(x),
        F(f) => crate::conv::round_int(f),
    }
}
/// `round(x, n)`: an int stays an int.
pub fn round_to(a: Num, n: i64) -> PyResult<Num> {
    Ok(match a {
        I(x) if n >= 0 => I(x),
        I(x) => {
            let p = 10i64.checked_pow((-n) as u32).unwrap_or(i64::MAX);
            let q = int::floordiv(x, p)?;
            let r = x - q * p;
            // Half to even.
            let up = 2 * r > p || (2 * r == p && q % 2 != 0);
            I(int::mul(q + up as i64, p)?)
        }
        F(f) => F(crate::conv::round_to(f, n)),
    })
}

fn cmp(a: Num, b: Num) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (I(x), I(y)) => Some(x.cmp(&y)),
        (I(x), F(y)) => int::cmp_float(x, y),
        (F(x), I(y)) => int::cmp_float(y, x).map(|o| o.reverse()),
        (F(x), F(y)) => x.partial_cmp(&y),
    }
}
pub fn eq(a: Num, b: Num) -> bool {
    cmp(a, b) == Some(std::cmp::Ordering::Equal)
}
pub fn ne(a: Num, b: Num) -> bool {
    !eq(a, b)
}
pub fn lt(a: Num, b: Num) -> bool {
    cmp(a, b) == Some(std::cmp::Ordering::Less)
}
pub fn le(a: Num, b: Num) -> bool {
    matches!(cmp(a, b), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal))
}
pub fn gt(a: Num, b: Num) -> bool {
    cmp(a, b) == Some(std::cmp::Ordering::Greater)
}
pub fn ge(a: Num, b: Num) -> bool {
    matches!(cmp(a, b), Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal))
}

impl crate::conv::PyFmt for Num {
    fn py_repr(&self) -> Str {
        match self {
            I(i) => crate::conv::str_int(*i),
            F(f) => crate::conv::str_float(*f),
        }
    }
}

impl crate::Truthy for Num {
    fn truthy(&self) -> bool {
        match self {
            I(i) => *i != 0,
            F(f) => *f != 0.0,
        }
    }
}

impl crate::coll::PyEq for Num {
    fn py_eq(&self, o: &Self) -> bool {
        eq(*self, *o)
    }
}

impl crate::coll::PyOrd for Num {
    fn py_cmp(&self, o: &Self) -> PyResult<std::cmp::Ordering> {
        // NaN compares as neither smaller nor larger (sorting keeps it in place).
        Ok(cmp(*self, *o).unwrap_or(std::cmp::Ordering::Equal))
    }
}

// Dict keys / set elements: `1` and `1.0` are the same key, as in Python.
impl PartialEq for Num {
    fn eq(&self, o: &Self) -> bool {
        eq(*self, *o) || matches!((self, o), (F(a), F(b)) if a.is_nan() && b.is_nan())
    }
}
impl Eq for Num {}
impl std::hash::Hash for Num {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        match *self {
            I(i) => i.hash(h),
            F(f) if f.fract() == 0.0 && f >= -9.2e18 && f <= 9.2e18 => (f as i64).hash(h),
            F(f) => f.to_bits().hash(h),
        }
    }
}

impl From<Num> for crate::fmt::Arg {
    fn from(n: Num) -> Self {
        match n {
            I(i) => crate::fmt::Arg::Int(i),
            F(f) => crate::fmt::Arg::Float(f),
        }
    }
}

#[cfg(feature = "python")]
impl crate::py::ToPy for Num {
    fn to_py(&self) -> PyResult<crate::py::Obj> {
        Ok(match self {
            I(i) => crate::py::int(*i),
            F(f) => crate::py::float(*f),
        })
    }
}
