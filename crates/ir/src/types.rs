/// Concrete IR types (design section 6). `Int` is the documented
/// fixed-width subset for M1: Python `int` restricted to i64, with checked
/// arithmetic that raises `OverflowError` instead of wrapping. Arbitrary
/// precision promotion is milestone M4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Ty {
    Bool,
    Int,
    Float,
    Str,
    None,
    /// A boxed CPython object (`rt::py::Obj`), used at Python boundaries and
    /// wherever hybrid mode boxes a value whose type is not statically known.
    Obj,
}

impl Ty {
    pub fn py_name(self) -> &'static str {
        match self {
            Ty::Bool => "bool",
            Ty::Int => "int",
            Ty::Float => "float",
            Ty::Str => "str",
            Ty::None => "None",
            Ty::Obj => "object",
        }
    }

    pub fn rust(self) -> &'static str {
        match self {
            Ty::Bool => "bool",
            Ty::Int => "i64",
            Ty::Float => "f64",
            Ty::Str => "rt::Str",
            Ty::None => "()",
            Ty::Obj => "rt::py::Obj",
        }
    }

    /// Short mangling component for specialized function names.
    pub fn mangle(self) -> &'static str {
        match self {
            Ty::Bool => "bool",
            Ty::Int => "int",
            Ty::Float => "float",
            Ty::Str => "str",
            Ty::None => "none",
            Ty::Obj => "obj",
        }
    }

    pub fn is_numeric(self) -> bool {
        matches!(self, Ty::Bool | Ty::Int | Ty::Float)
    }

    /// Zero-cost Rust initializer for a declared-but-not-yet-assigned slot.
    /// Only used for locals proven definitely-assigned before every read.
    pub fn rust_placeholder(self) -> &'static str {
        match self {
            Ty::Bool => "false",
            Ty::Int => "0",
            Ty::Float => "0.0",
            Ty::Str => "rt::Str::from(\"\")",
            Ty::None => "()",
            Ty::Obj => "rt::py::none()",
        }
    }

    /// Values that are cloned (not copied) when read.
    pub fn is_clone(self) -> bool {
        matches!(self, Ty::Str | Ty::Obj)
    }
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.py_name())
    }
}
