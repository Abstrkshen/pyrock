//! Generators: a function containing `yield` returns a [`Gen`]; its body
//! runs on its own stack and suspends at each `yield` with the value.

use crate::exc::{Exc, PyResult};
use crate::stackful::Co;
use corosensei::{CoroutineResult, Yielder};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

enum State<T> {
    Fresh(Box<dyn FnOnce() -> PyResult<()>>),
    Suspended(Co<(), T, PyResult<()>>),
    Running,
    Done,
}

/// A generator object yielding values of type `T`.
pub struct Gen<T> {
    name: &'static str,
    state: Rc<RefCell<State<T>>>,
}

impl<T> Clone for Gen<T> {
    fn clone(&self) -> Self {
        Gen { name: self.name, state: self.state.clone() }
    }
}

thread_local! {
    /// Yielders of the generator bodies currently running (innermost last),
    /// type-erased: `yield_::<T>` is only reached from a body yielding `T`.
    static CUR: RefCell<Vec<*const dyn Any>> = const { RefCell::new(Vec::new()) };
}

impl<T: 'static> Gen<T> {
    pub fn new(name: &'static str, body: impl FnOnce() -> PyResult<()> + 'static) -> Self {
        Gen { name, state: Rc::new(RefCell::new(State::Fresh(Box::new(body)))) }
    }

    /// `next(gen)`: the next value, or `None` when the body has returned.
    pub fn next(&self) -> PyResult<Option<T>> {
        let st = std::mem::replace(&mut *self.state.borrow_mut(), State::Running);
        let mut co = match st {
            State::Fresh(body) => Co::new(move |y: &Yielder<(), T>, _: ()| {
                let p: *const Yielder<(), T> = y;
                CUR.with(|c| c.borrow_mut().push(p as *const dyn Any));
                let r = body();
                CUR.with(|c| c.borrow_mut().pop());
                r
            })
            .map_err(|e| Exc::new("MemoryError", e))?,
            State::Suspended(co) => co,
            State::Running => {
                *self.state.borrow_mut() = State::Running;
                return Err(Exc::new("ValueError", "generator already executing"));
            }
            State::Done => {
                *self.state.borrow_mut() = State::Done;
                return Ok(None);
            }
        };
        match co.resume(()) {
            CoroutineResult::Yield(v) => {
                *self.state.borrow_mut() = State::Suspended(co);
                Ok(Some(v))
            }
            CoroutineResult::Return(r) => {
                *self.state.borrow_mut() = State::Done;
                r.map(|()| None)
            }
        }
    }

    /// Exhaust into a vector (`list(gen)`, `sum(gen)`, ...).
    pub fn to_vec(&self) -> PyResult<Vec<T>> {
        let mut out = Vec::new();
        while let Some(v) = self.next()? {
            out.push(v);
        }
        Ok(out)
    }
}

/// `yield v` inside a generator body yielding `T`.
pub fn yield_<T: 'static>(v: T) {
    let Some(p) = CUR.with(|c| c.borrow_mut().pop()) else {
        panic!("pyfast: `yield` outside a running generator");
    };
    // SAFETY: `p` is the yielder of the innermost running generator body,
    // which is the body executing this `yield` (bodies push their yielder on
    // entry and pop it while suspended), so its type is `Yielder<(), T>` and
    // it outlives this call.
    let y = unsafe { &*(p as *const Yielder<(), T>) };
    y.suspend(v);
    CUR.with(|c| c.borrow_mut().push(p));
}

/// Placeholder for not-yet-assigned locals (already exhausted).
impl<T> Default for Gen<T> {
    fn default() -> Self {
        Gen { name: "<unassigned>", state: Rc::new(RefCell::new(State::Done)) }
    }
}

impl<T> crate::conv::PyFmt for Gen<T> {
    fn py_repr(&self) -> crate::Str {
        format!("<generator object {} at {:#x}>", self.name, Rc::as_ptr(&self.state) as *const u8 as usize).into()
    }
}

impl<T> crate::Truthy for Gen<T> {
    fn truthy(&self) -> bool {
        true
    }
}

impl<T> crate::coll::PyEq for Gen<T> {
    fn py_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.state, &other.state)
    }
}

#[cfg(feature = "python")]
mod bridge {
    use super::Gen;
    use crate::exc::PyResult;
    use crate::py::{Obj, ToPy};
    use pyo3::prelude::*;

    /// A native generator seen from Python (an iterator).
    #[pyclass(unsendable, name = "native_generator")]
    struct PyGen {
        next: Box<dyn Fn() -> PyResult<Option<Obj>>>,
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
    }

    impl<T: ToPy + 'static> ToPy for Gen<T> {
        fn to_py(&self) -> PyResult<Obj> {
            let g = self.clone();
            let next = Box::new(move || -> PyResult<Option<Obj>> {
                match g.next()? {
                    Some(v) => Ok(Some(v.to_py()?)),
                    None => Ok(None),
                }
            });
            Python::attach(|py| Py::new(py, PyGen { next, name: self.name }).map(|o| Obj::from_py(o.into_any())))
                .map_err(|e| crate::exc::Exc::new("RuntimeError", e.to_string()))
        }
    }
}
