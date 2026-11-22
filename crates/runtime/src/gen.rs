//! Generators: a function containing `yield` returns a [`Gen`]; its body
//! runs on its own stack and suspends at each `yield` with the value.
//!
//! `Gen<T, S, R>` yields `T`, receives `S` from `send` (the value of the
//! `yield` expression; `None` for `next`), and returns `R` (the value of
//! `return v`, which `yield from` produces). A suspended generator is resumed
//! with a value or with an exception (`throw`, `close`), which is raised at
//! the `yield`, so the body's own `try`/`finally` handle it, as in CPython.

use crate::exc::{Exc, PyResult};
use crate::stackful::Co;
use corosensei::{CoroutineResult, Yielder};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

/// How a suspended generator is resumed.
pub enum Resume<S> {
    Send(S),
    Throw(Exc),
}

/// A value `send` can pass: `next` sends `None`, and a just-started
/// generator only accepts `None`.
pub trait SendValue: Default + 'static {
    fn is_none(&self) -> bool;
}
impl SendValue for () {
    fn is_none(&self) -> bool {
        true
    }
}
impl<T: 'static> SendValue for Option<T> {
    fn is_none(&self) -> bool {
        Option::is_none(self)
    }
}
#[cfg(feature = "python")]
impl SendValue for crate::py::Obj {
    fn is_none(&self) -> bool {
        crate::py::is_none(self)
    }
}

type Body<R> = Box<dyn FnOnce() -> PyResult<R>>;

enum State<T, S, R> {
    Fresh(Body<R>),
    Suspended(Co<Resume<S>, T, PyResult<R>>),
    Running,
    Done,
}

struct Inner<T: 'static, S: SendValue, R: 'static> {
    state: RefCell<State<T, S, R>>,
    ret: RefCell<Option<R>>,
}

/// A generator object.
pub struct Gen<T: 'static, S: SendValue = (), R: 'static = ()> {
    name: &'static str,
    inner: Rc<Inner<T, S, R>>,
}

impl<T: 'static, S: SendValue, R: 'static> Clone for Gen<T, S, R> {
    fn clone(&self) -> Self {
        Gen { name: self.name, inner: self.inner.clone() }
    }
}

thread_local! {
    /// Yielders of the generator bodies currently running (innermost last),
    /// type-erased: `yield_::<T, S>` is only reached from a body of that type.
    static CUR: RefCell<Vec<*const dyn Any>> = const { RefCell::new(Vec::new()) };
}

fn stop_iteration() -> Exc {
    Exc::new("StopIteration", "")
}

fn generator_exit() -> Exc {
    Exc::new("GeneratorExit", "")
}

impl<T: 'static, S: SendValue, R: 'static> Gen<T, S, R> {
    pub fn new(name: &'static str, body: impl FnOnce() -> PyResult<R> + 'static) -> Self {
        Gen { name, inner: Rc::new(Inner { state: RefCell::new(State::Fresh(Box::new(body))), ret: RefCell::new(None) }) }
    }

    /// Resume the body: `Some(value)` when it yields, `None` when it returns.
    pub fn resume(&self, input: Resume<S>) -> PyResult<Option<T>> {
        self.inner.resume(input)
    }

    /// `next(gen)`: the next value, or `None` when the body has returned.
    pub fn next(&self) -> PyResult<Option<T>> {
        self.inner.resume(Resume::Send(S::default()))
    }

    /// `gen.send(v)` (`StopIteration` when the body returns).
    pub fn send(&self, v: S) -> PyResult<T> {
        if !v.is_none() && matches!(*self.inner.state.borrow(), State::Fresh(_)) {
            return Err(Exc::new("TypeError", "can't send non-None value to a just-started generator"));
        }
        self.inner.resume(Resume::Send(v))?.ok_or_else(stop_iteration)
    }

    /// `gen.throw(exc)`: raised at the `yield` the body is suspended at.
    pub fn throw(&self, e: Exc) -> PyResult<T> {
        self.inner.resume(Resume::Throw(e))?.ok_or_else(stop_iteration)
    }

    /// `gen.close()`
    pub fn close(&self) -> PyResult<()> {
        self.inner.close()
    }

    /// Whether the body has started running.
    pub fn started(&self) -> bool {
        !matches!(*self.inner.state.borrow(), State::Fresh(_))
    }

    /// The value of the body's `return` (`yield from` produces it).
    pub fn take_ret(&self) -> Option<R> {
        self.inner.ret.borrow_mut().take()
    }

    /// Exhaust into a vector (`list(gen)`, `sum(gen)`, ...).
    pub fn to_vec(&self) -> PyResult<Vec<T>> {
        let mut out = Vec::new();
        while let Some(v) = self.next()? {
            out.push(v);
        }
        Ok(out)
    }

    /// A generator yielding `f(v)` for each `v` of this one (a type
    /// conversion); sends, throws and the return value pass through.
    pub fn map<U: 'static>(self, f: impl Fn(T) -> PyResult<U> + 'static) -> Gen<U, S, R> {
        let name = self.name;
        Gen::new(name, move || {
            let mut input = Resume::Send(S::default());
            loop {
                match self.resume(input)? {
                    Some(v) => input = yield_raw::<U, S>(f(v)?),
                    None => return Ok(self.take_ret().expect("generator return value")),
                }
            }
        })
    }
}

impl<T: 'static, S: SendValue, R: 'static> Inner<T, S, R> {
    fn resume(&self, input: Resume<S>) -> PyResult<Option<T>> {
        let st = std::mem::replace(&mut *self.state.borrow_mut(), State::Running);
        let mut co = match st {
            State::Fresh(body) => {
                // Throwing into a generator that has not started ends it.
                if let Resume::Throw(e) = input {
                    *self.state.borrow_mut() = State::Done;
                    return Err(e);
                }
                Co::new(move |y: &Yielder<Resume<S>, T>, _: Resume<S>| {
                    let p: *const Yielder<Resume<S>, T> = y;
                    CUR.with(|c| c.borrow_mut().push(p as *const dyn Any));
                    let r = body();
                    CUR.with(|c| c.borrow_mut().pop());
                    r
                })
                .map_err(|e| Exc::new("MemoryError", e))?
            }
            State::Suspended(co) => co,
            State::Running => {
                *self.state.borrow_mut() = State::Running;
                return Err(Exc::new("ValueError", "generator already executing"));
            }
            State::Done => {
                *self.state.borrow_mut() = State::Done;
                return match input {
                    Resume::Throw(e) => Err(e),
                    Resume::Send(_) => Ok(None),
                };
            }
        };
        match co.resume(input) {
            CoroutineResult::Yield(v) => {
                *self.state.borrow_mut() = State::Suspended(co);
                Ok(Some(v))
            }
            CoroutineResult::Return(r) => {
                *self.state.borrow_mut() = State::Done;
                match r {
                    Ok(v) => {
                        *self.ret.borrow_mut() = Some(v);
                        Ok(None)
                    }
                    // PEP 479: a StopIteration escaping the body is a RuntimeError.
                    Err(e) if e.name() == "StopIteration" => Err(Exc::new("RuntimeError", "generator raised StopIteration")),
                    Err(e) => Err(e),
                }
            }
        }
    }

    fn close(&self) -> PyResult<()> {
        if !matches!(*self.state.borrow(), State::Suspended(_)) {
            if !matches!(*self.state.borrow(), State::Running) {
                *self.state.borrow_mut() = State::Done;
            }
            return Ok(());
        }
        match self.resume(Resume::Throw(generator_exit())) {
            Ok(Some(_)) => Err(Exc::new("RuntimeError", "generator ignored GeneratorExit")),
            Ok(None) => Ok(()),
            Err(e) if matches!(e.name(), "GeneratorExit" | "StopIteration") => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// An unfinished generator that is dropped is closed, as CPython does: its
/// `finally` blocks run (errors are ignored, as CPython only reports them).
impl<T: 'static, S: SendValue, R: 'static> Drop for Inner<T, S, R> {
    fn drop(&mut self) {
        if matches!(*self.state.borrow(), State::Suspended(_)) {
            let _ = self.close();
        }
    }
}

/// `yield v` inside a generator body yielding `T` and receiving `S`: the
/// sent value, or the thrown exception raised here.
pub fn yield_<T: 'static, S: 'static>(v: T) -> PyResult<S> {
    match yield_raw::<T, S>(v) {
        Resume::Send(s) => Ok(s),
        Resume::Throw(e) => Err(e),
    }
}

pub(crate) fn yield_raw<T: 'static, S: 'static>(v: T) -> Resume<S> {
    let Some(p) = CUR.with(|c| c.borrow_mut().pop()) else {
        panic!("pyfast: `yield` outside a running generator");
    };
    // SAFETY: `p` is the yielder of the innermost running generator body,
    // which is the body executing this `yield` (bodies push their yielder on
    // entry and pop it while suspended), so its type is
    // `Yielder<Resume<S>, T>` and it outlives this call.
    let y = unsafe { &*(p as *const Yielder<Resume<S>, T>) };
    let r = y.suspend(v);
    CUR.with(|c| c.borrow_mut().push(p));
    r
}

/// `yield from sub` where `sub` is a native generator: its values are
/// yielded, sends and throws are forwarded to it, and its return value is
/// the result.
pub fn yield_from<T: 'static, SO: 'static, U: 'static, SI: SendValue, R: 'static>(
    sub: &Gen<U, SI, R>,
    conv: impl Fn(U) -> PyResult<T>,
    conv_send: impl Fn(SO) -> PyResult<SI>,
) -> PyResult<R> {
    let mut input = Resume::Send(SI::default());
    loop {
        match sub.resume(input)? {
            Some(v) => {
                input = match yield_raw::<T, SO>(conv(v)?) {
                    Resume::Send(s) => Resume::Send(conv_send(s)?),
                    Resume::Throw(e) => Resume::Throw(e),
                }
            }
            None => return Ok(sub.take_ret().expect("generator return value")),
        }
    }
}

impl<T: 'static> Gen<T> {
    /// A generator over the items of a vector.
    pub fn from_vec(v: Vec<T>) -> Gen<T> {
        Gen::new("<iterator>", move || {
            for x in v {
                yield_::<T, ()>(x)?;
            }
            Ok(())
        })
    }
}

/// `enumerate(gen, start)`, lazily.
pub fn enumerate<T: 'static, S: SendValue, R: 'static>(g: Gen<T, S, R>, start: i64) -> Gen<(i64, T)> {
    Gen::new("enumerate", move || {
        let mut i = start;
        while let Some(v) = g.next()? {
            yield_::<(i64, T), ()>((i, v))?;
            i = i.checked_add(1).ok_or_else(Exc::overflow)?;
        }
        Ok(())
    })
}

/// `zip(a, b)` with a generator, lazily (stops at the shortest).
pub fn zip2<A: 'static, SA: SendValue, RA: 'static, B: 'static, SB: SendValue, RB: 'static>(a: Gen<A, SA, RA>, b: Gen<B, SB, RB>) -> Gen<(A, B)> {
    Gen::new("zip", move || {
        while let Some(x) = a.next()? {
            let Some(y) = b.next()? else { break };
            yield_::<(A, B), ()>((x, y))?;
        }
        Ok(())
    })
}

/// `zip(a, b, c)` with a generator, lazily.
#[allow(clippy::type_complexity)]
pub fn zip3<A: 'static, SA: SendValue, RA: 'static, B: 'static, SB: SendValue, RB: 'static, C: 'static, SC: SendValue, RC: 'static>(
    a: Gen<A, SA, RA>,
    b: Gen<B, SB, RB>,
    c: Gen<C, SC, RC>,
) -> Gen<(A, B, C)> {
    Gen::new("zip", move || {
        while let Some(x) = a.next()? {
            let Some(y) = b.next()? else { break };
            let Some(z) = c.next()? else { break };
            yield_::<(A, B, C), ()>((x, y, z))?;
        }
        Ok(())
    })
}

/// Placeholder for not-yet-assigned locals (already exhausted).
impl<T: 'static, S: SendValue, R: 'static> Default for Gen<T, S, R> {
    fn default() -> Self {
        Gen { name: "<unassigned>", inner: Rc::new(Inner { state: RefCell::new(State::Done), ret: RefCell::new(None) }) }
    }
}

impl<T: 'static, S: SendValue, R: 'static> crate::conv::PyFmt for Gen<T, S, R> {
    fn py_repr(&self) -> crate::Str {
        format!("<generator object {} at {:#x}>", self.name, Rc::as_ptr(&self.inner) as *const u8 as usize).into()
    }
}

impl<T: 'static, S: SendValue, R: 'static> crate::Truthy for Gen<T, S, R> {
    fn truthy(&self) -> bool {
        true
    }
}

impl<T: 'static, S: SendValue, R: 'static> crate::coll::PyEq for Gen<T, S, R> {
    fn py_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

#[cfg(feature = "python")]
mod bridge {
    use super::{Gen, SendValue};
    use crate::exc::PyResult;
    use crate::py::{Obj, ToPy};
    use pyo3::prelude::*;

    /// A native generator seen from Python (an iterator).
    #[pyclass(unsendable, name = "native_generator")]
    struct PyGen {
        next: Box<dyn Fn() -> PyResult<Option<Obj>>>,
        throw: Box<dyn Fn(crate::exc::Exc) -> PyResult<Option<Obj>>>,
        close: Box<dyn Fn() -> PyResult<()>>,
        name: &'static str,
    }

    #[pymethods]
    impl PyGen {
        fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
            slf
        }

        fn __next__(&self) -> pyo3::PyResult<Option<Py<PyAny>>> {
            match (self.next)() {
                Ok(v) => Ok(v.map(|o| o.into_py())),
                Err(e) => Err(crate::py::to_pyerr(e)),
            }
        }

        fn __repr__(&self) -> String {
            format!("<generator object {}>", self.name)
        }

        fn throw(&self, e: Py<PyAny>) -> pyo3::PyResult<Py<PyAny>> {
            let exc = crate::py::raise(&Obj::from_py(e));
            match (self.throw)(exc) {
                Ok(Some(v)) => Ok(v.into_py()),
                Ok(None) => Err(pyo3::exceptions::PyStopIteration::new_err(())),
                Err(e) => Err(crate::py::to_pyerr(e)),
            }
        }

        fn close(&self) -> pyo3::PyResult<()> {
            (self.close)().map_err(crate::py::to_pyerr)
        }
    }

    impl<T: ToPy + 'static, S: SendValue, R: 'static> ToPy for Gen<T, S, R> {
        fn to_py(&self) -> PyResult<Obj> {
            let g = self.clone();
            let next = Box::new(move || -> PyResult<Option<Obj>> {
                match g.next()? {
                    Some(v) => Ok(Some(v.to_py()?)),
                    None => Ok(None),
                }
            });
            let g2 = self.clone();
            let throw = Box::new(move |e| -> PyResult<Option<Obj>> {
                match g2.resume(super::Resume::Throw(e))? {
                    Some(v) => Ok(Some(v.to_py()?)),
                    None => Ok(None),
                }
            });
            let g3 = self.clone();
            let close = Box::new(move || g3.close());
            Python::attach(|py| Py::new(py, PyGen { next, throw, close, name: self.name }).map(|o| Obj::from_py(o.into_any())))
                .map_err(|e| crate::exc::Exc::new("RuntimeError", e.to_string()))
        }
    }
}

/// A `for` loop over a generator owns it: when the loop ends (also by
/// `break`), the generator is dropped and so closed, as in CPython.
pub struct GenIter<T: 'static, S: SendValue, R: 'static>(pub Gen<T, S, R>);

impl<T: 'static, S: SendValue, R: 'static> Iterator for GenIter<T, S, R> {
    type Item = PyResult<T>;
    fn next(&mut self) -> Option<PyResult<T>> {
        match self.0.next() {
            Ok(Some(v)) => Some(Ok(v)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}
