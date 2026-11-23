//! Python `float` operations (ports of CPython's floatobject.c rules).

use crate::exc::{Exc, PyResult};

#[inline]
pub fn truediv(a: f64, b: f64) -> PyResult<f64> {
    if b == 0.0 {
        return Err(Exc::zero_div("float division by zero"));
    }
    Ok(a / b)
}

fn divmod(vx: f64, wx: f64) -> (f64, f64) {
    let mut m = vx % wx; // C fmod
    let mut div = (vx - m) / wx;
    if m != 0.0 {
        if (wx < 0.0) != (m < 0.0) {
            m += wx;
            div -= 1.0;
        }
    } else {
        m = 0.0f64.copysign(wx);
    }
    let floordiv = if div != 0.0 {
        let mut f = div.floor();
        if div - f > 0.5 {
            f += 1.0;
        }
        f
    } else {
        0.0f64.copysign(vx / wx)
    };
    (floordiv, m)
}

pub fn floordiv(a: f64, b: f64) -> PyResult<f64> {
    if b == 0.0 {
        return Err(Exc::zero_div("float floor division by zero"));
    }
    Ok(divmod(a, b).0)
}

pub fn modulo(a: f64, b: f64) -> PyResult<f64> {
    if b == 0.0 {
        return Err(Exc::zero_div("float modulo"));
    }
    let mut m = a % b;
    if m != 0.0 {
        if (b < 0.0) != (m < 0.0) {
            m += b;
        }
    } else {
        m = 0.0f64.copysign(b);
    }
    Ok(m)
}

pub fn pow(a: f64, b: f64) -> PyResult<f64> {
    if b == 0.0 {
        return Ok(1.0);
    }
    if a.is_nan() {
        return Ok(a);
    }
    if b.is_nan() {
        return Ok(if a == 1.0 { 1.0 } else { b });
    }
    if a == 0.0 && b < 0.0 && b.is_finite() {
        return Err(Exc::zero_div("0.0 cannot be raised to a negative power"));
    }
    if a < 0.0 && a.is_finite() && b.is_finite() && b.fract() != 0.0 {
        return Err(Exc::new(
            "NotImplementedError",
            "negative float ** fractional power produces a complex number (pyrock: complex is not supported)",
        ));
    }
    let r = a.powf(b);
    if r.is_infinite() && a.is_finite() && b.is_finite() {
        return Err(Exc::new("OverflowError", "(34, 'Numerical result out of range')"));
    }
    Ok(r)
}

/// Python `repr(float)` / `str(float)`: shortest round-trip digits, fixed
/// notation for exponents in [-4, 16), scientific otherwise.
pub fn repr(v: f64) -> String {
    if v.is_nan() {
        return "nan".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    // Rust's `{:e}` yields the shortest round-trip digits: "d.ddde<exp>".
    let s = format!("{:e}", v.abs());
    let (mant, exp) = s.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let sign = if v < 0.0 { "-" } else { "" };
    if (-4..16).contains(&exp) {
        let n = digits.len() as i32;
        let body = if exp < 0 {
            format!("0.{}{}", "0".repeat((-exp - 1) as usize), digits)
        } else if exp + 1 >= n {
            format!("{}{}.0", digits, "0".repeat((exp + 1 - n) as usize))
        } else {
            let (a, b) = digits.split_at((exp + 1) as usize);
            format!("{a}.{b}")
        };
        format!("{sign}{body}")
    } else {
        let m = if digits.len() > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits };
        let es = if exp < 0 { "-" } else { "+" };
        format!("{sign}{m}e{es}{:02}", exp.abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_matches_cpython() {
        let cases = [
            (2.5, "2.5"),
            (1.0, "1.0"),
            (0.1, "0.1"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (123456789012345680.0, "1.2345678901234568e+17"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.5e-5, "1.5e-05"),
            (-0.0, "-0.0"),
            (1.0 / 3.0, "0.3333333333333333"),
            (6.25, "6.25"),
            (1e300 * 10.0, "1e+301"),
            (5e-324, "5e-324"),
        ];
        for (v, want) in cases {
            assert_eq!(repr(v), want, "repr({v:?})");
        }
    }

    #[test]
    fn floor_and_mod() {
        assert_eq!(floordiv(-7.0, 2.0), Ok(-4.0));
        assert_eq!(modulo(-7.0, 2.0), Ok(1.0));
        assert_eq!(modulo(7.0, -2.0), Ok(-1.0));
        assert!(modulo(-0.0, 1.0).unwrap().is_sign_positive());
    }
}
