//! pyfast runtime: Python-compatible semantics for natively compiled code.
//!
//! Generated programs `use pyfast_runtime as rt;` and call these helpers
//! instead of raw Rust operators wherever the two differ (overflow, floor
//! division, modulo signs, float formatting, exceptions, ...).
//!
//! Int subset (M1): Python `int` is represented as `i64`. Any operation
//! whose exact result does not fit raises `OverflowError` instead of
//! wrapping or silently changing type. Arbitrary precision is milestone M4.

pub mod aio;
pub mod coll;
pub mod conv;
pub mod exc;
pub mod ext;
pub mod float;
pub mod gen;
pub mod fmt;
pub mod int;
pub mod io;
#[cfg(feature = "python")]
pub mod py;
pub mod stackful;
pub mod string;

pub use coll::{Dict, List, Never, Set, Tup, Tuple0};
pub use exc::{Exc, Flow, PyResult};
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

/// A module-level variable kept in a global cell.
pub type Global<T> = std::thread::LocalKey<std::cell::RefCell<Option<T>>>;

/// Read a global cell (NameError while unbound, as in CPython).
#[inline]
pub fn gget<T: Clone + 'static>(cell: &'static Global<T>, name: &str) -> PyResult<T> {
    cell.with(|c| c.borrow().clone()).ok_or_else(|| unbound_global(name))
}

/// Assign a global cell.
#[inline]
pub fn gset<T: 'static>(cell: &'static Global<T>, v: T) {
    cell.with(|c| *c.borrow_mut() = Some(v));
}

pub fn unbound_global(name: &str) -> Exc {
    Exc::new("NameError", format!("name '{name}' is not defined"))
}

/// Python truthiness for native values (`if x:`, `not x`, `bool(x)`).
pub trait Truthy {
    fn truthy(&self) -> bool;
}

impl Truthy for i64 {
    fn truthy(&self) -> bool {
        *self != 0
    }
}

impl Truthy for f64 {
    fn truthy(&self) -> bool {
        *self != 0.0
    }
}

impl Truthy for bool {
    fn truthy(&self) -> bool {
        *self
    }
}

impl Truthy for Str {
    fn truthy(&self) -> bool {
        !self.is_empty()
    }
}

impl Truthy for () {
    fn truthy(&self) -> bool {
        false
    }
}

impl<T: Truthy> Truthy for Option<T> {
    fn truthy(&self) -> bool {
        self.as_ref().is_some_and(|v| v.truthy())
    }
}

impl Truthy for Exc {
    fn truthy(&self) -> bool {
        true
    }
}

/// `bool(x)` for a native value.
pub fn truthy<T: Truthy + ?Sized>(v: &T) -> bool {
    v.truthy()
}

/// A `T | None` value used where a `T` is required: CPython raises
/// TypeError (the message is the one CPython gives at that site).
#[inline]
pub fn unopt<T>(v: Option<T>, msg: &str) -> PyResult<T> {
    v.ok_or_else(|| Exc::new("TypeError", msg.to_string()))
}

/// A closure cell: a variable shared between a function and the nested
/// functions that capture it (`None` while unbound).
pub type Cell<T> = std::rc::Rc<std::cell::RefCell<Option<T>>>;

/// A fresh, unbound cell.
#[inline]
pub fn cell<T>() -> Cell<T> {
    std::rc::Rc::new(std::cell::RefCell::new(None))
}

/// Read a cell (NameError for an unbound free variable, as CPython).
#[inline]
pub fn cget<T: Clone>(c: &Cell<T>, name: &str) -> PyResult<T> {
    c.borrow().clone().ok_or_else(|| Exc::new("NameError", format!("cannot access free variable '{name}' where it is not associated with a value in enclosing scope")))
}

#[inline]
pub fn cset<T>(c: &Cell<T>, v: T) {
    *c.borrow_mut() = Some(v);
}

/// The environment of a function value (a tuple of the cells it captured).
#[derive(Clone, Default)]
pub struct Env<T>(pub T);

impl<T> conv::PyFmt for Env<T> {
    fn py_repr(&self) -> Str {
        Str::from(format!("<function at {:#x}>", self as *const _ as usize))
    }
}

impl<T> Truthy for Env<T> {
    fn truthy(&self) -> bool {
        true
    }
}

impl<T> coll::PyEq for Env<T> {
    fn py_eq(&self, o: &Self) -> bool {
        std::ptr::eq(self, o)
    }
}

/// Per-instance cache of the Python wrapper of a native class instance (a
/// weak reference, so the wrapper keeps its identity while alive).
#[cfg(feature = "python")]
pub type PyCache = std::cell::RefCell<Option<pyo3::Py<pyo3::types::PyWeakrefReference>>>;
#[cfg(not(feature = "python"))]
#[derive(Default)]
pub struct PyCache;
