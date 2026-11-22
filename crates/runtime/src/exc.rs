/// A raised Python exception (native representation). When the CPython
/// bridge is present this maps to and from `PyErr`.
#[derive(Clone, Debug, PartialEq)]
pub struct Exc {
    /// Exception type name as CPython prints it (`ValueError`,
    /// `json.decoder.JSONDecodeError`, ...).
    pub kind: std::borrow::Cow<'static, str>,
    pub msg: String,
}

pub type PyResult<T> = Result<T, Exc>;

impl Exc {
    pub fn new(kind: impl Into<std::borrow::Cow<'static, str>>, msg: impl Into<String>) -> Exc {
        Exc { kind: kind.into(), msg: msg.into() }
    }

    pub fn overflow() -> Exc {
        Exc::new("OverflowError", "int result does not fit in the 64-bit int subset (pyfast: arbitrary precision is not implemented yet)")
    }

    pub fn zero_div(msg: &str) -> Exc {
        Exc::new("ZeroDivisionError", msg)
    }
}

impl std::fmt::Display for Exc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.msg.is_empty() {
            f.write_str(&self.kind)
        } else {
            write!(f, "{}: {}", self.kind, self.msg)
        }
    }
}
