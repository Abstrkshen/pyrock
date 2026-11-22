//! pyfast runtime: Python-compatible semantics for natively compiled code.
//!
//! Generated programs `use pyfast_runtime as rt;` and call these helpers
//! instead of raw Rust operators wherever the two differ (overflow, floor
//! division, modulo signs, float formatting, exceptions, ...).
//!
//! Int subset (M1): Python `int` is represented as `i64`. Any operation
//! whose exact result does not fit raises `OverflowError` instead of
//! wrapping or silently changing type. Arbitrary precision is milestone M4.

pub mod conv;
pub mod exc;
pub mod ext;
pub mod float;
pub mod int;
pub mod io;
#[cfg(feature = "python")]
pub mod py;
pub mod string;

pub use exc::{Exc, PyResult};
pub use io::{print, run_main};

/// Immutable Python `str` value; cheap clones preserve value semantics.
pub type Str = std::rc::Rc<str>;

/// Recursion guard matching CPython's default limit.
pub struct Frame(());

thread_local! {
    static DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub const RECURSION_LIMIT: u32 = 1000;

#[inline]
pub fn enter() -> PyResult<Frame> {
    DEPTH.with(|d| {
        let n = d.get() + 1;
        if n > RECURSION_LIMIT {
            return Err(Exc::new("RecursionError", "maximum recursion depth exceeded"));
        }
        d.set(n);
        Ok(Frame(()))
    })
}

impl Drop for Frame {
    #[inline]
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get() - 1));
    }
}

#[inline]
pub fn truthy_int(v: i64) -> bool {
    v != 0
}

#[inline]
pub fn truthy_float(v: f64) -> bool {
    v != 0.0
}

#[inline]
pub fn truthy_str(v: &Str) -> bool {
    !v.is_empty()
}

pub fn unbound_local(name: &str) -> Exc {
    Exc::new("UnboundLocalError", format!("cannot access local variable '{name}' where it is not associated with a value"))
}

/// Run a natively compiled module's body only once (Python import semantics).
pub fn first_import(flag: &std::sync::atomic::AtomicBool) -> bool {
    !flag.swap(true, std::sync::atomic::Ordering::Relaxed)
}

pub fn unbound_global(name: &str) -> Exc {
    Exc::new("NameError", format!("name '{name}' is not defined"))
}
