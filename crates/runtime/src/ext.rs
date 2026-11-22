//! Conversions at the Rust/C module boundary (`import x` → `x.rs` / `x.c`).

use crate::exc::{Exc, PyResult};
use crate::Str;
use std::ffi::{c_char, CStr, CString};

/// Python int → fixed-width integer parameter, with CPython's overflow error.
pub fn int_to<T: TryFrom<i64>>(v: i64, ty: &str) -> PyResult<T> {
    T::try_from(v).map_err(|_| Exc::new("OverflowError", format!("Python int too large to convert to {ty}")))
}

/// Fixed-width integer result → Python int (i64 subset).
pub fn int_from<T: TryInto<i64>>(v: T) -> PyResult<i64> {
    v.try_into().map_err(|_| Exc::overflow())
}

/// Python str → C `const char *` (kept alive by the returned CString).
pub fn cstring(s: &Str) -> PyResult<CString> {
    CString::new(s.as_bytes()).map_err(|_| Exc::new("ValueError", "embedded null character"))
}

/// C `const char *` result → Python str (copied; the C side keeps ownership).
///
/// # Safety
/// `p` must be NULL or point to a NUL-terminated string valid for this call.
pub unsafe fn from_cstr(p: *const c_char) -> PyResult<Str> {
    if p.is_null() {
        return Err(Exc::new("SystemError", "C function returned NULL for a `const char *` result"));
    }
    Ok(Str::from(CStr::from_ptr(p).to_string_lossy().as_ref()))
}

/// Error from a Rust `Result<T, E>` function.
pub fn rust_err<E: std::fmt::Display>(e: E) -> Exc {
    Exc::new("RuntimeError", e.to_string())
}

thread_local! {
    /// An exception raised inside a callback called by Rust/C code (which
    /// cannot propagate it): re-raised when that call returns.
    static STASH: std::cell::RefCell<Option<Exc>> = const { std::cell::RefCell::new(None) };
}

/// Keep the first exception raised by a callback.
pub fn stash(e: Exc) {
    STASH.with(|s| {
        let mut s = s.borrow_mut();
        if s.is_none() {
            *s = Some(e);
        }
    });
}

/// After a Rust/C call that received callbacks: re-raise a stashed exception.
pub fn check() -> PyResult<()> {
    match STASH.with(|s| s.borrow_mut().take()) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
