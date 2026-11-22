//! Async generators (`async def` with `yield`).
//!
//! The body runs as a generator ([`Gen`]) that yields events: a value for
//! the consumer, or a Python awaitable to await. The consumer's `anext`
//! awaits those on its own stack (inside its coroutine, where suspending to
//! the event loop is possible) and resumes the body with the result, until
//! the body yields a value or returns.

use crate::exc::{Exc, PyResult};
use crate::gen::{yield_raw, Gen, Resume};
use crate::py::Obj;
use std::any::Any;
use std::marker::PhantomData;

/// What an async generator body produces when it suspends.
pub enum Event {
    Value(Box<dyn Any>),
    Await(Obj),
}

/// An async generator yielding `T`.
pub struct AGen<T: 'static> {
    g: Gen<Event, Obj, ()>,
    _t: PhantomData<T>,
}

impl<T: 'static> Clone for AGen<T> {
    fn clone(&self) -> Self {
        AGen { g: self.g.clone(), _t: PhantomData }
    }
}

impl<T: 'static> Default for AGen<T> {
    fn default() -> Self {
        AGen { g: Gen::default(), _t: PhantomData }
    }
}

impl<T: 'static> AGen<T> {
    pub fn new(name: &'static str, body: impl FnOnce() -> PyResult<()> + 'static) -> Self {
        AGen {
            g: Gen::new(name, move || {
                crate::aio::push_agen();
                let r = body();
                crate::aio::pop_agen();
                r
            }),
            _t: PhantomData,
        }
    }

    /// Run the body until it yields a value (`async for` / `__anext__`),
    /// awaiting what it awaits on the way. `None`: it returned.
    pub fn anext(&self) -> PyResult<Option<T>> {
        self.drive(Resume::Send(Obj::default()))
    }

    fn drive(&self, mut input: Resume<Obj>) -> PyResult<Option<T>> {
        loop {
            match self.g.resume(input)? {
                None => return Ok(None),
                Some(Event::Value(b)) => return Ok(Some(*b.downcast::<T>().expect("async generator value type"))),
                Some(Event::Await(o)) => {
                    input = match crate::aio::await_obj(o) {
                        Ok(v) => Resume::Send(v),
                        Err(e) => Resume::Throw(e),
                    }
                }
            }
        }
    }

    /// `await agen.aclose()`
    pub fn aclose(&self) -> PyResult<()> {
        match self.drive(Resume::Throw(Exc::new("GeneratorExit", ""))) {
            Ok(Some(_)) => Err(Exc::new("RuntimeError", "async generator ignored GeneratorExit")),
            Ok(None) => Ok(()),
            Err(e) if matches!(e.name(), "GeneratorExit" | "StopAsyncIteration") => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// `yield v` in an async generator body.
pub fn yield_<T: 'static>(v: T) -> PyResult<()> {
    crate::aio::pop_agen();
    let r = yield_raw::<Event, Obj>(Event::Value(Box::new(v)));
    crate::aio::push_agen();
    match r {
        Resume::Send(_) => Ok(()),
        Resume::Throw(e) => Err(e),
    }
}

/// `await obj` reached inside an async generator body: hand the awaitable to
/// the consumer, which awaits it and sends back the result.
pub(crate) fn await_in_body(o: Obj) -> PyResult<Obj> {
    match yield_raw::<Event, Obj>(Event::Await(o)) {
        Resume::Send(v) => Ok(v),
        Resume::Throw(e) => Err(e),
    }
}

impl<T: 'static> crate::conv::PyFmt for AGen<T> {
    fn py_repr(&self) -> crate::Str {
        crate::Str::from("<async_generator object>")
    }
}

impl<T: 'static> crate::Truthy for AGen<T> {
    fn truthy(&self) -> bool {
        true
    }
}

impl<T: 'static> crate::coll::PyEq for AGen<T> {
    fn py_eq(&self, o: &Self) -> bool {
        self.g.py_eq(&o.g)
    }
}

mod bridge {
    use super::AGen;
    use crate::aio::Coro;
    use crate::exc::{Exc, PyResult};
    use crate::py::{Obj, ToPy};
    use pyo3::prelude::*;

    /// A native async generator seen from Python (`async for`, `aclose`).
    #[pyclass(unsendable, name = "native_async_generator")]
    struct PyAGen {
        anext: Box<dyn Fn() -> PyResult<Obj>>,
        aclose: Box<dyn Fn() -> PyResult<Obj>>,
    }

    #[pymethods]
    impl PyAGen {
        fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
            slf
        }

        fn __anext__(&self) -> pyo3::PyResult<Py<PyAny>> {
            (self.anext)().map(|o| o.into_py()).map_err(crate::py::to_pyerr)
        }

        fn aclose(&self) -> pyo3::PyResult<Py<PyAny>> {
            (self.aclose)().map(|o| o.into_py()).map_err(crate::py::to_pyerr)
        }
    }

    impl<T: ToPy + 'static> ToPy for AGen<T> {
        fn to_py(&self) -> PyResult<Obj> {
            let a = self.clone();
            // Each `__anext__` is a coroutine running the body to its next value.
            let anext = Box::new(move || -> PyResult<Obj> {
                let a = a.clone();
                Coro::new("anext", move || match a.anext()? {
                    Some(v) => v.to_py(),
                    None => Err(Exc::new("StopAsyncIteration", "")),
                })
                .to_py()
            });
            let c = self.clone();
            let aclose = Box::new(move || -> PyResult<Obj> {
                let c = c.clone();
                Coro::new("aclose", move || {
                    c.aclose()?;
                    Ok(crate::py::none())
                })
                .to_py()
            });
            Python::attach(|py| Py::new(py, PyAGen { anext, aclose }).map(|o| Obj::from_py(o.into_any())))
                .map_err(|e| Exc::new("RuntimeError", e.to_string()))
        }
    }
}
