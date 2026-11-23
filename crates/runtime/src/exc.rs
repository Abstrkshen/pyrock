//! Exceptions. A raised Python exception is `Err(Exc)`; every compiled
//! function returns `PyResult<T>` and propagates with `?`, so `try` blocks
//! only need to intercept the `Err`.

use std::any::Any;
use std::borrow::Cow;
use std::rc::Rc;

/// A raised Python exception (native representation). When the CPython
/// bridge is present it maps to and from `PyErr`.
#[derive(Clone)]
pub struct Exc {
    /// Exception type name as CPython prints it (`ValueError`,
    /// `json.decoder.JSONDecodeError`, ...).
    pub kind: Cow<'static, str>,
    pub msg: String,
    /// The original exception object when it came from CPython (an
    /// `rt::py::Obj`), so `except` can use `isinstance` and re-raising it
    /// into Python keeps its identity. Native classes keep their instance here.
    pub origin: Option<Rc<dyn Any>>,
}

pub type PyResult<T> = Result<T, Exc>;

impl PartialEq for Exc {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.msg == other.msg
    }
}

impl std::fmt::Debug for Exc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Exc({:?}, {:?})", self.kind, self.msg)
    }
}

impl Default for Exc {
    fn default() -> Exc {
        Exc::new("Exception", "")
    }
}

impl Exc {
    pub fn new(kind: impl Into<Cow<'static, str>>, msg: impl Into<String>) -> Exc {
        Exc { kind: kind.into(), msg: msg.into(), origin: None }
    }

    pub fn overflow() -> Exc {
        Exc::new("OverflowError", "int result does not fit in the 64-bit int subset (pyfast: arbitrary precision is not implemented yet)")
    }

    pub fn zero_div(msg: &str) -> Exc {
        Exc::new("ZeroDivisionError", msg)
    }

    /// Unqualified class name (`JSONDecodeError` for `json.decoder.JSONDecodeError`).
    pub fn name(&self) -> &str {
        self.kind.rsplit('.').next().unwrap_or(&self.kind)
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

/// CPython's builtin exception hierarchy: (class, base).
pub const HIERARCHY: &[(&str, &str)] = &[
    ("Exception", "BaseException"),
    ("BaseExceptionGroup", "BaseException"),
    ("GeneratorExit", "BaseException"),
    ("KeyboardInterrupt", "BaseException"),
    ("SystemExit", "BaseException"),
    ("ArithmeticError", "Exception"),
    ("FloatingPointError", "ArithmeticError"),
    ("OverflowError", "ArithmeticError"),
    ("ZeroDivisionError", "ArithmeticError"),
    ("AssertionError", "Exception"),
    ("AttributeError", "Exception"),
    ("FrozenInstanceError", "AttributeError"),
    ("BufferError", "Exception"),
    ("EOFError", "Exception"),
    ("ExceptionGroup", "Exception"),
    ("ImportError", "Exception"),
    ("ModuleNotFoundError", "ImportError"),
    ("LookupError", "Exception"),
    ("IndexError", "LookupError"),
    ("KeyError", "LookupError"),
    ("MemoryError", "Exception"),
    ("NameError", "Exception"),
    ("UnboundLocalError", "NameError"),
    ("OSError", "Exception"),
    ("BlockingIOError", "OSError"),
    ("ChildProcessError", "OSError"),
    ("ConnectionError", "OSError"),
    ("BrokenPipeError", "ConnectionError"),
    ("ConnectionAbortedError", "ConnectionError"),
    ("ConnectionRefusedError", "ConnectionError"),
    ("ConnectionResetError", "ConnectionError"),
    ("FileExistsError", "OSError"),
    ("FileNotFoundError", "OSError"),
    ("InterruptedError", "OSError"),
    ("IsADirectoryError", "OSError"),
    ("NotADirectoryError", "OSError"),
    ("PermissionError", "OSError"),
    ("ProcessLookupError", "OSError"),
    ("TimeoutError", "OSError"),
    ("ReferenceError", "Exception"),
    ("RuntimeError", "Exception"),
    ("NotImplementedError", "RuntimeError"),
    ("PythonFinalizationError", "RuntimeError"),
    ("RecursionError", "RuntimeError"),
    ("StopAsyncIteration", "Exception"),
    ("StopIteration", "Exception"),
    ("SyntaxError", "Exception"),
    ("IndentationError", "SyntaxError"),
    ("TabError", "IndentationError"),
    ("SystemError", "Exception"),
    ("TypeError", "Exception"),
    ("ValueError", "Exception"),
    ("UnicodeError", "ValueError"),
    ("UnicodeDecodeError", "UnicodeError"),
    ("UnicodeEncodeError", "UnicodeError"),
    ("UnicodeTranslateError", "UnicodeError"),
    ("Warning", "Exception"),
    ("BytesWarning", "Warning"),
    ("DeprecationWarning", "Warning"),
    ("EncodingWarning", "Warning"),
    ("FutureWarning", "Warning"),
    ("ImportWarning", "Warning"),
    ("PendingDeprecationWarning", "Warning"),
    ("ResourceWarning", "Warning"),
    ("RuntimeWarning", "Warning"),
    ("SyntaxWarning", "Warning"),
    ("UnicodeWarning", "Warning"),
    ("UserWarning", "Warning"),
];

/// Whether `name` is a builtin exception class (or one of its aliases).
pub fn is_builtin(name: &str) -> bool {
    name == "BaseException" || matches!(name, "IOError" | "EnvironmentError") || HIERARCHY.iter().any(|(c, _)| *c == name)
}

fn canonical(name: &str) -> &str {
    match name {
        "IOError" | "EnvironmentError" => "OSError",
        n => n,
    }
}

/// Exception classes defined by the compiled program: (class, base).
static USER: crate::Shared<Vec<(&'static str, &'static str)>> = crate::Shared::new(Vec::new());

/// Register the program's own exception classes (called from `main`).
pub fn register(classes: &[(&'static str, &'static str)]) {
    USER.with(|u| u.extend_from_slice(classes));
}

fn base_of(name: &str) -> Option<&'static str> {
    if let Some((_, b)) = HIERARCHY.iter().find(|(c, _)| *c == name) {
        return Some(b);
    }
    USER.with(|u| u.iter().find(|(c, _)| *c == name).map(|(_, b)| *b))
}

/// Whether class `kind` is `base` or derives from it.
pub fn is_subclass(kind: &str, base: &str) -> bool {
    let base = canonical(base);
    let mut cur = canonical(kind).to_string();
    for _ in 0..64 {
        if cur == base {
            return true;
        }
        match base_of(&cur) {
            Some(b) => cur = b.to_string(),
            None => return false,
        }
    }
    false
}

/// `except <base>:` for a native exception class name.
pub fn matches(e: &Exc, base: &str) -> bool {
    #[cfg(feature = "python")]
    if let Some(r) = crate::py::exc_matches_builtin(e, base) {
        return r;
    }
    is_subclass(e.name(), base)
}

/// The native instance carried by an exception of a compiled exception class.
pub fn downcast<T: Clone + 'static>(e: &Exc) -> Option<T> {
    e.origin.as_ref().and_then(|o| o.downcast_ref::<T>()).cloned()
}

/// `ValueError(arg)` / `KeyError(arg)`: the message `str(exc)` produces.
/// `KeyError` shows the key's repr; the others show `str(arg)`.
pub fn new1(kind: &'static str, arg: crate::Str, arg_repr: crate::Str) -> Exc {
    if kind == "KeyError" {
        Exc::new(kind, arg_repr.to_string())
    } else {
        Exc::new(kind, arg.to_string())
    }
}

/// `raise` with no active exception.
pub fn no_active() -> Exc {
    Exc::new("RuntimeError", "No active exception to reraise")
}

/// `assert cond[, msg]`
pub fn assertion(msg: Option<crate::Str>) -> Exc {
    Exc::new("AssertionError", msg.map(|m| m.to_string()).unwrap_or_default())
}

/// How control left a `try` body (it runs in a closure so that `?` can be
/// intercepted): falling off the end, or a `return`/`break`/`continue`
/// that must be re-issued after any `finally`.
pub enum Flow<T> {
    Normal,
    Return(T),
    Break,
    Continue,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchy() {
        assert!(is_subclass("ZeroDivisionError", "ArithmeticError"));
        assert!(is_subclass("KeyError", "LookupError"));
        assert!(is_subclass("KeyError", "Exception"));
        assert!(is_subclass("FileNotFoundError", "IOError"));
        assert!(!is_subclass("KeyboardInterrupt", "Exception"));
        assert!(is_subclass("KeyboardInterrupt", "BaseException"));
        assert!(!is_subclass("ValueError", "TypeError"));
        assert!(matches(&Exc::new("json.decoder.JSONDecodeError", "x"), "JSONDecodeError"));
    }
}
