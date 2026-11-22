//! `async def` / `await`.
//!
//! Calling a compiled `async def` makes a [`Coro`]: the body, not yet run.
//! `await` on a `Coro` runs the body inline (as CPython does: awaiting a
//! coroutine does not go through the event loop).
//!
//! When a `Coro` reaches CPython (`asyncio.run(main())`, `gather`,
//! `create_task`), it becomes a real Python coroutine: a small Python driver
//! (`_drive`) that resumes the body on its own stack (a stackful coroutine).
//! When the body awaits a Python awaitable (`asyncio.sleep`, a task, a
//! future), the body suspends and hands the awaitable to the driver, which
//! awaits it in the event loop and resumes the body with the result (or
//! throws the exception into it). Everything runs on the event loop's thread.

use crate::exc::{Exc, PyResult};
use std::cell::RefCell;
use std::rc::Rc;

/// A coroutine of a compiled `async def`: the body to run once.
pub struct Coro<T> {
    name: &'static str,
    body: Rc<RefCell<Option<Box<dyn FnOnce() -> PyResult<T>>>>>,
}

impl<T> Clone for Coro<T> {
    fn clone(&self) -> Self {
        Coro { name: self.name, body: self.body.clone() }
    }
}

impl<T> Coro<T> {
    pub fn new(name: &'static str, f: impl FnOnce() -> PyResult<T> + 'static) -> Self {
        Coro { name, body: Rc::new(RefCell::new(Some(Box::new(f)))) }
    }

    /// `await coro`: run the body to completion (it may suspend inside, at
    /// awaits of Python objects).
    pub fn run(&self) -> PyResult<T> {
        let f = self.body.borrow_mut().take();
        match f {
            Some(f) => f(),
            None => Err(Exc::new("RuntimeError", "cannot reuse already awaited coroutine")),
        }
    }
}

/// Placeholder for not-yet-assigned locals (never awaited).
impl<T> Default for Coro<T> {
    fn default() -> Self {
        Coro { name: "<unassigned>", body: Rc::new(RefCell::new(None)) }
    }
}

impl<T> crate::conv::PyFmt for Coro<T> {
    fn py_repr(&self) -> crate::Str {
        format!("<coroutine object {} at {:#x}>", self.name, Rc::as_ptr(&self.body) as *const u8 as usize).into()
    }
}

impl<T> crate::Truthy for Coro<T> {
    fn truthy(&self) -> bool {
        true
    }
}

impl<T> crate::coll::PyEq for Coro<T> {
    fn py_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.body, &other.body)
    }
}

#[cfg(feature = "python")]
pub use bridge::{aenter, aexit, aiter, await_obj, AIter};

#[cfg(not(feature = "python"))]
pub fn await_obj<O>(_: O) -> PyResult<O> {
    Err(Exc::new("RuntimeError", "await of a Python object needs CPython"))
}

#[cfg(feature = "python")]
mod bridge {
    use super::Coro;
    use crate::exc::{Exc, PyResult};
    use crate::py::{Obj, ToPy};
    use crate::stackful::Co as Stackful;
    use corosensei::{CoroutineResult, Yielder};
    use pyo3::prelude::*;
    use std::cell::RefCell;

    /// What the driver sends into a suspended body: the awaited value, or the
    /// exception the await raised.
    type In = Result<Obj, Exc>;
    type Co = Stackful<In, Obj, PyResult<Obj>>;

    thread_local! {
        /// Yielders of the coroutine bodies currently running (innermost last).
        static CUR: RefCell<Vec<*const Yielder<In, Obj>>> = const { RefCell::new(Vec::new()) };
        static DRIVER: RefCell<Option<Obj>> = const { RefCell::new(None) };
    }

    const DRIVER_SRC: &std::ffi::CStr = c"
async def _drive(step):
    done, v = step.send(None, None)
    while not done:
        try:
            r = await v
        except BaseException as e:
            done, v = step.send(None, e)
        else:
            done, v = step.send(r, None)
    return v
";

    /// `await obj` inside a compiled coroutine: suspend the body until the
    /// event loop has awaited `obj`.
    pub fn await_obj(o: Obj) -> PyResult<Obj> {
        let Some(y) = CUR.with(|c| c.borrow_mut().pop()) else {
            return Err(Exc::new("RuntimeError", "pyfast: `await` of a Python object outside a coroutine run by an event loop"));
        };
        crate::io::flush();
        // SAFETY: the yielder belongs to the body running on this stack; it
        // lives until that body returns, which cannot happen before this call
        // does.
        let r = unsafe { &*y }.suspend(o);
        CUR.with(|c| c.borrow_mut().push(y));
        r
    }

    /// `async with cm`: await `__aenter__`.
    pub fn aenter(cm: &Obj) -> PyResult<Obj> {
        await_obj(crate::py::aenter_call(cm)?)
    }

    /// `async with` exit: await `__aexit__`; whether it suppressed `err`.
    pub fn aexit(cm: &Obj, err: Option<&Exc>) -> PyResult<bool> {
        let r = await_obj(crate::py::exit_call(cm, "__aexit__", err)?)?;
        crate::py::is_true(&r)
    }

    /// The iterator of an `async for`.
    pub struct AIter(Obj);

    /// `async for x in o`: `type(o).__aiter__(o)`.
    pub fn aiter(o: &Obj) -> PyResult<AIter> {
        Ok(AIter(crate::py::call_method(o, "__aiter__", vec![])?))
    }

    impl AIter {
        /// Await `__anext__`; `None` at `StopAsyncIteration`.
        pub fn next(&mut self) -> PyResult<Option<Obj>> {
            let aw = crate::py::call_method(&self.0, "__anext__", vec![])?;
            match await_obj(aw) {
                Ok(v) => Ok(Some(v)),
                Err(e) if crate::exc::matches(&e, "StopAsyncIteration") => Ok(None),
                Err(e) => Err(e),
            }
        }
    }

    /// One compiled coroutine as seen by the Python driver.
    #[pyclass(unsendable, name = "native_coroutine_step")]
    struct Step {
        co: Option<Co>,
    }

    #[pymethods]
    impl Step {
        /// Resume the body with a value or an exception: `(True, result)` when
        /// it finished, `(False, awaitable)` when it awaits a Python object.
        fn send(&mut self, py: Python<'_>, value: Py<PyAny>, exc: Option<Py<PyAny>>) -> pyo3::PyResult<(bool, Py<PyAny>)> {
            let Some(co) = self.co.as_mut() else {
                return Err(pyo3::exceptions::PyRuntimeError::new_err("cannot reuse already awaited coroutine"));
            };
            let input: In = match exc {
                Some(e) => Err(crate::py::raise(&Obj::from_py(e))),
                None => Ok(Obj::from_py(value)),
            };
            let _ = py;
            let r = co.resume(input);
            match r {
                CoroutineResult::Yield(aw) => Ok((false, aw.into_py())),
                CoroutineResult::Return(res) => {
                    self.co = None;
                    match res {
                        Ok(v) => Ok((true, v.into_py())),
                        Err(e) => Err(crate::py::to_pyerr(e)),
                    }
                }
            }
        }
    }

    impl<T: ToPy + 'static> ToPy for Coro<T> {
        fn to_py(&self) -> PyResult<Obj> {
            let c = self.clone();
            let co: Co = Stackful::new(move |y: &Yielder<In, Obj>, _start: In| {
                CUR.with(|s| s.borrow_mut().push(y as *const _));
                let r = c.run().and_then(|v| v.to_py());
                CUR.with(|s| s.borrow_mut().pop());
                r
            })
            .map_err(|e| Exc::new("MemoryError", e))?;
            let driver = DRIVER.with(|d| -> PyResult<Obj> {
                if let Some(o) = d.borrow().as_ref() {
                    return Ok(o.clone());
                }
                let o = crate::py::run_module_attr(DRIVER_SRC, c"pyfast_aio.py", c"pyfast_aio", "_drive")?;
                *d.borrow_mut() = Some(o.clone());
                Ok(o)
            })?;
            let step = Python::attach(|py| Py::new(py, Step { co: Some(co) }).map(|s| Obj::from_py(s.into_any())))
                .map_err(|e| Exc::new("RuntimeError", e.to_string()))?;
            crate::py::call(&driver, vec![step], vec![])
        }
    }
}
