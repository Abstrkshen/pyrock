//! Python `int` operations on the i64 subset.

use crate::exc::{Exc, PyResult};

#[inline]
pub fn add(a: i64, b: i64) -> PyResult<i64> {
    a.checked_add(b).ok_or_else(Exc::overflow)
}

#[inline]
pub fn sub(a: i64, b: i64) -> PyResult<i64> {
    a.checked_sub(b).ok_or_else(Exc::overflow)
}

#[inline]
pub fn mul(a: i64, b: i64) -> PyResult<i64> {
    a.checked_mul(b).ok_or_else(Exc::overflow)
}

#[inline]
pub fn neg(a: i64) -> PyResult<i64> {
    a.checked_neg().ok_or_else(Exc::overflow)
}

#[inline]
pub fn abs(a: i64) -> PyResult<i64> {
    a.checked_abs().ok_or_else(Exc::overflow)
}

#[inline]
pub fn invert(a: i64) -> i64 {
    !a
}

/// Floor division (rounds toward negative infinity).
#[inline]
pub fn floordiv(a: i64, b: i64) -> PyResult<i64> {
    if b == 0 {
        return Err(Exc::zero_div("integer division or modulo by zero"));
    }
    let q = a.checked_div(b).ok_or_else(Exc::overflow)?;
    if (a % b != 0) && ((a < 0) != (b < 0)) {
        Ok(q - 1)
    } else {
        Ok(q)
    }
}

/// Modulo with the sign of the divisor.
#[inline]
pub fn modulo(a: i64, b: i64) -> PyResult<i64> {
    if b == 0 {
        return Err(Exc::zero_div("integer modulo by zero"));
    }
    let r = a.checked_rem(b).unwrap_or(0); // i64::MIN % -1 == 0
    if r != 0 && ((r < 0) != (b < 0)) {
        Ok(r + b)
    } else {
        Ok(r)
    }
}

/// True division `int / int`, correctly rounded like CPython.
pub fn truediv(a: i64, b: i64) -> PyResult<f64> {
    if b == 0 {
        return Err(Exc::zero_div("division by zero"));
    }
    const EXACT: u64 = 1 << 53;
    if a.unsigned_abs() <= EXACT && b.unsigned_abs() <= EXACT {
        return Ok(a as f64 / b as f64);
    }
    let neg = (a < 0) != (b < 0);
    let (ua, ub) = (a.unsigned_abs() as u128, b.unsigned_abs() as u128);
    if ua == 0 {
        return Ok(if neg { -0.0 } else { 0.0 });
    }
    // Scale the dividend to 127 bits so the quotient carries >= 63
    // significant bits, fold the remainder into a sticky bit, then let the
    // (round-to-nearest-even) u128 -> f64 conversion round exactly once.
    let k = 127 - (128 - ua.leading_zeros()) as i32;
    let num = ua << k;
    let q = num / ub;
    let sticky = !num.is_multiple_of(ub) as u128;
    let v = ((q << 1) | sticky) as f64 * 2f64.powi(-(k + 1));
    Ok(if neg { -v } else { v })
}

/// `int ** int` for non-negative exponents.
pub fn pow(a: i64, b: i64) -> PyResult<i64> {
    if b < 0 {
        return Err(Exc::new(
            "NotImplementedError",
            "int ** negative int produces a float, which this int specialization cannot hold (pyfast M1 subset)",
        ));
    }
    match a {
        0 => return Ok(if b == 0 { 1 } else { 0 }),
        1 => return Ok(1),
        -1 => return Ok(if b % 2 == 0 { 1 } else { -1 }),
        _ => {}
    }
    let exp = u32::try_from(b).map_err(|_| Exc::overflow())?;
    a.checked_pow(exp).ok_or_else(Exc::overflow)
}

#[inline]
pub fn bitand(a: i64, b: i64) -> i64 {
    a & b
}

#[inline]
pub fn bitor(a: i64, b: i64) -> i64 {
    a | b
}

#[inline]
pub fn bitxor(a: i64, b: i64) -> i64 {
    a ^ b
}

pub fn lshift(a: i64, b: i64) -> PyResult<i64> {
    if b < 0 {
        return Err(Exc::new("ValueError", "negative shift count"));
    }
    if a == 0 {
        return Ok(0);
    }
    if b >= 64 {
        return Err(Exc::overflow());
    }
    let r = a << b;
    if (r >> b) != a {
        return Err(Exc::overflow());
    }
    Ok(r)
}

pub fn rshift(a: i64, b: i64) -> PyResult<i64> {
    if b < 0 {
        return Err(Exc::new("ValueError", "negative shift count"));
    }
    Ok(if b >= 64 { if a < 0 { -1 } else { 0 } } else { a >> b })
}

/// Exact comparison of an int with a float (no lossy conversion).
pub fn cmp_float(i: i64, f: f64) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering::*;
    if f.is_nan() {
        return None;
    }
    const TWO63: f64 = 9_223_372_036_854_775_808.0;
    if f >= TWO63 {
        return Some(Less);
    }
    if f < -TWO63 {
        return Some(Greater);
    }
    let t = f.trunc();
    let ti = t as i64; // exact: |t| < 2^63
    match i.cmp(&ti) {
        Equal => {
            let frac = f - t;
            Some(if frac > 0.0 { Less } else if frac < 0.0 { Greater } else { Equal })
        }
        o => Some(o),
    }
}

// Exact int-vs-float comparisons (`i op f`). NaN compares unequal to everything.
pub fn lt_f(i: i64, f: f64) -> bool {
    cmp_float(i, f) == Some(std::cmp::Ordering::Less)
}
pub fn le_f(i: i64, f: f64) -> bool {
    matches!(cmp_float(i, f), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal))
}
pub fn gt_f(i: i64, f: f64) -> bool {
    cmp_float(i, f) == Some(std::cmp::Ordering::Greater)
}
pub fn ge_f(i: i64, f: f64) -> bool {
    matches!(cmp_float(i, f), Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal))
}
pub fn eq_f(i: i64, f: f64) -> bool {
    cmp_float(i, f) == Some(std::cmp::Ordering::Equal)
}
pub fn ne_f(i: i64, f: f64) -> bool {
    !eq_f(i, f)
}

/// `range(start, stop, step)` iteration.
pub fn range(start: i64, stop: i64, step: i64) -> PyResult<impl Iterator<Item = i64>> {
    if step == 0 {
        return Err(Exc::new("ValueError", "range() arg 3 must not be zero"));
    }
    let mut cur = start;
    Ok(std::iter::from_fn(move || {
        let more = if step > 0 { cur < stop } else { cur > stop };
        if !more {
            return None;
        }
        let v = cur;
        match cur.checked_add(step) {
            Some(n) => cur = n,
            None => cur = stop, // next value would pass i64 bounds, hence `stop`
        }
        Some(v)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_semantics() {
        assert_eq!(floordiv(-7, 2), Ok(-4));
        assert_eq!(floordiv(7, -2), Ok(-4));
        assert_eq!(modulo(-7, 2), Ok(1));
        assert_eq!(modulo(7, -2), Ok(-1));
        assert_eq!(modulo(i64::MIN, -1), Ok(0));
        assert!(floordiv(i64::MIN, -1).is_err());
    }

    #[test]
    fn truediv_rounding() {
        assert_eq!(truediv(1, 3), Ok(1.0 / 3.0));
        // 2**62 + 1 / 3 must be correctly rounded, not double-rounded.
        let a = (1i64 << 62) + 1;
        assert_eq!(truediv(a, 3).unwrap(), 1.5372286728091292e18); // CPython: (2**62+1)/3
        assert_eq!(truediv(i64::MAX, 1).unwrap(), 9.223372036854776e18);
        assert_eq!(truediv(1, i64::MAX).unwrap(), 1.0842021724855044e-19);
    }

    #[test]
    fn shifts() {
        assert_eq!(lshift(1, 62), Ok(1 << 62));
        assert!(lshift(1, 63).is_err());
        assert_eq!(rshift(-1, 100), Ok(-1));
    }

    #[test]
    fn int_float_compare() {
        use std::cmp::Ordering::*;
        assert_eq!(cmp_float(9007199254740993, 9007199254740992.0), Some(Greater));
        assert_eq!(cmp_float(1, 1.5), Some(Less));
        assert_eq!(cmp_float(-1, -1.5), Some(Greater));
        assert_eq!(cmp_float(3, 3.0), Some(Equal));
    }
}
