//! CPython bridge (design section 8), built on PyO3.
//!
//! * One interpreter per process, initialised by `init` with an explicit
//!   `sys.path` captured at build time.
//! * `Obj` owns a `Py<PyAny>`; every operation attaches to the interpreter
//!   only for its own duration, so long native loops never hold it.
//! * Python exceptions become `Exc` with CPython's qualified type name.
//! * Native stdout is flushed before every bridge call and the embedded
//!   interpreter runs unbuffered, so output from both sides stays ordered.

use crate::exc::{Exc, PyResult};
use crate::Str;
use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use std::sync::Mutex;

/// An owned reference to a CPython object.
pub struct Obj(Py<PyAny>);

impl Clone for Obj {
    fn clone(&self) -> Self {
        Python::attach(|py| Obj(self.0.clone_ref(py)))
    }
}

fn to_exc(py: Python<'_>, e: PyErr) -> Exc {
    let ty = e.get_type(py);
    let name = ty.qualname().map(|s| s.to_string()).unwrap_or_else(|_| "Exception".into());
    let module = ty.module().map(|s| s.to_string()).unwrap_or_default();
    let kind = if module.is_empty() || module == "builtins" { name } else { format!("{module}.{name}") };
    let msg = e.value(py).str().map(|s| s.to_string()).unwrap_or_default();
    let mut x = Exc::new(kind, msg);
    x.origin = Some(std::rc::Rc::new(Obj(e.value(py).clone().into_any().unbind())));
    x
}

/// The Python exception object carried by `e`, if it came from CPython.
fn origin(e: &Exc) -> Option<&Obj> {
    e.origin.as_ref().and_then(|o| o.downcast_ref::<Obj>())
}

/// `except <builtin name>` for an exception that came from CPython:
/// decided by `isinstance` (None when `e` is native or `base` is not a builtin).
pub(crate) fn exc_matches_builtin(e: &Exc, base: &str) -> Option<bool> {
    let o = origin(e)?;
    Python::attach(|py| {
        let cls = py.import("builtins").ok()?.getattr(base).ok()?;
        o.bind(py).is_instance(&cls).ok()
    })
}

/// Python exception instance for `e` (the original object, or a new
/// instance of the builtin class named by `e.kind`).
pub fn exc_obj(e: &Exc) -> PyResult<Obj> {
    if let Some(o) = origin(e) {
        return Ok(o.clone());
    }
    with(|py| {
        let builtins = py.import("builtins")?;
        let cls = match builtins.getattr(e.name()) {
            Ok(c) if crate::exc::is_builtin(e.name()) => c,
            _ => builtins.getattr("Exception")?,
        };
        if e.msg.is_empty() {
            return cls.call0().map(own);
        }
        // KeyError's message is the key's repr: recover the key itself.
        if e.name() == "KeyError" {
            if let Ok(k) = py.import("ast")?.getattr("literal_eval")?.call1((e.msg.as_str(),)) {
                return cls.call1((k,)).map(own);
            }
        }
        cls.call1((e.msg.as_str(),)).map(own)
    })
}

/// `except <python class or tuple>` (e.g. `json.JSONDecodeError`).
pub fn exc_matches(e: &Exc, cls: &Obj) -> PyResult<bool> {
    let o = exc_obj(e)?;
    with(|py| o.bind(py).is_instance(cls.bind(py)))
}

/// `raise <python object>`: an exception instance or class.
pub fn raise(o: &Obj) -> Exc {
    Python::attach(|py| to_exc(py, PyErr::from_value(o.bind(py).clone())))
}

/// `with` statement: `type(cm).__enter__(cm)`.
pub fn enter(cm: &Obj) -> PyResult<Obj> {
    with(|py| {
        let b = cm.bind(py);
        b.get_type().getattr("__enter__")?.call1((b,)).map(own)
    })
}

/// `with` statement exit. `err` is the exception leaving the body, if any;
/// returns whether `__exit__` suppressed it.
pub fn exit(cm: &Obj, err: Option<&Exc>) -> PyResult<bool> {
    let exc = match err {
        Some(e) => Some(exc_obj(e)?),
        None => None,
    };
    with(|py| {
        let b = cm.bind(py);
        let exit = b.get_type().getattr("__exit__")?;
        let r = match exc {
            None => exit.call1((b, py.None(), py.None(), py.None()))?,
            Some(x) => {
                let v = x.bind(py);
                exit.call1((b, v.get_type(), v, v.getattr("__traceback__")?))?
            }
        };
        r.is_truthy()
    })
}

/// Run `f` attached to the interpreter, mapping Python errors to `Exc`.
#[inline]
fn with<R>(f: impl FnOnce(Python<'_>) -> pyo3::PyResult<R>) -> PyResult<R> {
    crate::io::flush();
    Python::attach(|py| f(py).map_err(|e| to_exc(py, e)))
}

impl Obj {
    fn bind<'py>(&self, py: Python<'py>) -> &Bound<'py, PyAny> {
        self.0.bind(py)
    }
}

fn own(b: Bound<'_, PyAny>) -> Obj {
    Obj(b.unbind())
}

/// Start the embedded interpreter. `paths` are prepended to `sys.path`
/// (entry directory first, then the build interpreter's search path).
pub fn init(script: &str, paths: &[&str]) {
    // SAFETY: called once at program start, before any other thread exists.
    unsafe { std::env::set_var("PYTHONUNBUFFERED", "1") };
    Python::initialize();
    let r = with(|py| {
        let sys = py.import("sys")?;
        let sys_path = sys.getattr("path")?;
        for (i, p) in paths.iter().enumerate() {
            if !sys_path.contains(*p)? {
                sys_path.call_method1("insert", (i, *p))?;
            }
        }
        let argv: Vec<String> = std::iter::once(script.to_string()).chain(std::env::args().skip(1)).collect();
        sys.setattr("argv", argv)?;
        Ok(())
    });
    if let Err(e) = r {
        eprintln!("pyfast: failed to initialise embedded CPython: {e}");
        std::process::exit(1);
    }
}

// ---- boxing native values ----

pub fn int(v: i64) -> Obj {
    Python::attach(|py| own(v.into_pyobject(py).unwrap().into_any()))
}

pub fn float(v: f64) -> Obj {
    Python::attach(|py| own(v.into_pyobject(py).unwrap().into_any()))
}

pub fn bool_(v: bool) -> Obj {
    Python::attach(|py| own(v.into_pyobject(py).unwrap().to_owned().into_any()))
}

pub fn str_(v: &str) -> Obj {
    Python::attach(|py| own(v.into_pyobject(py).unwrap().into_any()))
}

pub fn none() -> Obj {
    Python::attach(|py| Obj(py.None()))
}

// ---- unboxing ----

/// `int`-typed parameter (uses `__index__`, like CPython's C API).
pub fn to_i64(o: &Obj) -> PyResult<i64> {
    with(|py| o.bind(py).extract::<i64>())
}

pub fn to_f64(o: &Obj) -> PyResult<f64> {
    with(|py| o.bind(py).extract::<f64>())
}

/// Exact `bool` required (for `bool` parameters of Rust/C functions).
pub fn to_bool(o: &Obj) -> PyResult<bool> {
    with(|py| o.bind(py).extract::<bool>())
}

/// Exact `str` required (for string parameters of Rust/C functions).
pub fn to_str(o: &Obj) -> PyResult<Str> {
    with(|py| o.bind(py).extract::<String>().map(Str::from))
}

/// Unbox a value flowing into an annotated slot. The object must be
/// exactly the annotated type (`bool` is rejected for `int`), so the native
/// value prints and behaves exactly like the original object.
pub fn unbox_annotated<T: Unboxable>(o: &Obj, ann: &str) -> PyResult<T> {
    with(|py| {
        let b = o.bind(py);
        let ok = match ann {
            "int" => b.is_exact_instance_of::<pyo3::types::PyInt>(),
            "str" => b.is_instance_of::<pyo3::types::PyString>(),
            _ => b.is_exact_instance_of::<pyo3::types::PyBool>(),
        };
        if !ok {
            let got = b.get_type().qualname().map(|s| s.to_string()).unwrap_or_default();
            return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "annotation `{ann}` expects {ann}, got {got} (pyfast enforces annotations where it unboxes Python objects)"
            )));
        }
        T::unbox(b)
    })
}

pub trait Unboxable: Sized {
    fn unbox(b: &Bound<'_, PyAny>) -> pyo3::PyResult<Self>;
}

impl Unboxable for i64 {
    fn unbox(b: &Bound<'_, PyAny>) -> pyo3::PyResult<Self> {
        b.extract::<i64>().map_err(|_| pyo3::exceptions::PyOverflowError::new_err("int does not fit in the 64-bit int subset"))
    }
}

impl Unboxable for Str {
    fn unbox(b: &Bound<'_, PyAny>) -> pyo3::PyResult<Self> {
        b.extract::<String>().map(Str::from)
    }
}

impl Unboxable for bool {
    fn unbox(b: &Bound<'_, PyAny>) -> pyo3::PyResult<Self> {
        b.extract::<bool>()
    }
}

/// `str(o)`
pub fn str_of(o: &Obj) -> PyResult<Str> {
    with(|py| Ok(Str::from(o.bind(py).str()?.to_cow()?.as_ref())))
}

/// `int(o)` (within the i64 subset)
pub fn int_of(o: &Obj) -> PyResult<i64> {
    with(|py| {
        let v = py.get_type::<pyo3::types::PyInt>().call1((o.bind(py),))?;
        v.extract::<i64>()
    })
}

/// `float(o)`
pub fn float_of(o: &Obj) -> PyResult<f64> {
    with(|py| py.get_type::<pyo3::types::PyFloat>().call1((o.bind(py),))?.extract::<f64>())
}

pub fn truthy(o: &Obj) -> PyResult<bool> {
    with(|py| o.bind(py).is_truthy())
}

pub fn len(o: &Obj) -> PyResult<i64> {
    with(|py| Ok(o.bind(py).len()? as i64))
}

// ---- object protocol ----

pub fn getattr(o: &Obj, name: &str) -> PyResult<Obj> {
    with(|py| o.bind(py).getattr(name).map(own))
}

pub fn setattr(o: &Obj, name: &str, v: Obj) -> PyResult<()> {
    with(|py| o.bind(py).setattr(name, v.0))
}

pub fn getitem(o: &Obj, k: Obj) -> PyResult<Obj> {
    with(|py| o.bind(py).get_item(k.0).map(own))
}

pub fn setitem(o: &Obj, k: Obj, v: Obj) -> PyResult<()> {
    with(|py| o.bind(py).set_item(k.0, v.0))
}

/// `f(*args, **kwargs)`
pub fn call(f: &Obj, args: Vec<Obj>, kwargs: Vec<(&str, Obj)>) -> PyResult<Obj> {
    with(|py| {
        let args = PyTuple::new(py, args.into_iter().map(|a| a.0))?;
        if kwargs.is_empty() {
            return f.bind(py).call1(args).map(own);
        }
        let kw = PyDict::new(py);
        for (k, v) in kwargs {
            kw.set_item(k, v.0)?;
        }
        f.bind(py).call(args, Some(&kw)).map(own)
    })
}

pub fn builtin(name: &str) -> PyResult<Obj> {
    with(|py| py.import("builtins")?.getattr(name).map(own))
}

pub fn list(items: Vec<Obj>) -> PyResult<Obj> {
    with(|py| PyList::new(py, items.into_iter().map(|a| a.0)).map(|l| own(l.into_any())))
}

pub fn tuple(items: Vec<Obj>) -> PyResult<Obj> {
    with(|py| PyTuple::new(py, items.into_iter().map(|a| a.0)).map(|l| own(l.into_any())))
}

pub fn dict(items: Vec<(Obj, Obj)>) -> PyResult<Obj> {
    with(|py| {
        let d = PyDict::new(py);
        for (k, v) in items {
            d.set_item(k.0, v.0)?;
        }
        Ok(own(d.into_any()))
    })
}

/// Binary operator by Python name: `add`, `sub`, `mul`, `truediv`,
/// `floordiv`, `mod`, `pow`, `and_`, `or_`, `xor`, `lshift`, `rshift`,
/// or an in-place variant (`iadd`, ...), via the `operator` module.
pub fn binop(op: &str, a: Obj, b: Obj) -> PyResult<Obj> {
    with(|py| {
        let (a, b) = (a.0.bind(py), b.0.bind(py));
        match op {
            "add" => a.add(b),
            "sub" => a.sub(b),
            "mul" => a.mul(b),
            "truediv" => a.div(b),
            "floordiv" => a.floor_div(b),
            "mod" => a.rem(b),
            "and_" => a.bitand(b),
            "or_" => a.bitor(b),
            "xor" => a.bitxor(b),
            "lshift" => a.lshift(b),
            "rshift" => a.rshift(b),
            other => py.import("operator")?.getattr(other)?.call1((a, b)),
        }
        .map(own)
    })
}

/// Unary operator via the `operator` module (`neg`, `pos`, `invert`, `abs`).
pub fn unary(op: &str, a: Obj) -> PyResult<Obj> {
    with(|py| py.import("operator")?.getattr(op)?.call1((a.0,)).map(own))
}

fn cmp_op(op: &str) -> CompareOp {
    match op {
        "lt" => CompareOp::Lt,
        "le" => CompareOp::Le,
        "gt" => CompareOp::Gt,
        "ge" => CompareOp::Ge,
        "ne" => CompareOp::Ne,
        _ => CompareOp::Eq,
    }
}

/// Rich comparison returning whatever the object returns.
pub fn compare(op: &str, a: Obj, b: Obj) -> PyResult<Obj> {
    with(|py| a.0.bind(py).rich_compare(b.0, cmp_op(op)).map(own))
}

/// Rich comparison reduced to a bool (chained comparisons, `if`).
pub fn compare_bool(op: &str, a: Obj, b: Obj) -> PyResult<bool> {
    with(|py| a.0.bind(py).rich_compare(b.0, cmp_op(op))?.is_truthy())
}

/// `item in container`
pub fn contains(item: Obj, container: Obj) -> PyResult<bool> {
    with(|py| container.0.bind(py).contains(item.0))
}

/// `a is b`
pub fn is(a: &Obj, b: &Obj) -> bool {
    Python::attach(|py| a.bind(py).is(b.bind(py)))
}

/// Iterator over a Python iterable.
pub struct Iter(Py<PyAny>);

pub fn iter(o: &Obj) -> PyResult<Iter> {
    with(|py| Ok(Iter(o.bind(py).try_iter()?.into_any().unbind())))
}

impl Iter {
    pub fn next(&mut self) -> PyResult<Option<Obj>> {
        with(|py| {
            let it = self.0.bind(py).cast::<pyo3::types::PyIterator>().map_err(PyErr::from)?.clone();
            match it.into_iter().next() {
                None => Ok(None),
                Some(v) => v.map(|v| Some(own(v))),
            }
        })
    }
}

/// A module-level name bound by a CPython import, filled when the import
/// statement executes (preserving import order and side effects).
pub struct Slot {
    binding: &'static str,
    cell: Mutex<Option<Obj>>,
}

impl Slot {
    pub const fn new(binding: &'static str) -> Slot {
        Slot { binding, cell: Mutex::new(None) }
    }

    /// `import a.b` (binds `a`, `bind_top`) or `import a.b as x` (binds `a.b`).
    pub fn import(&self, module: &str, bind_top: bool) -> PyResult<()> {
        let m = with(|py| {
            let m = py.import(module)?;
            if bind_top && module.contains('.') {
                let top = module.split('.').next().unwrap();
                return py.import(top).map(|t| own(t.into_any()));
            }
            Ok(own(m.into_any()))
        })?;
        *self.cell.lock().unwrap() = Some(m);
        Ok(())
    }

    /// `from module import attr`
    pub fn import_from(&self, module: &str, attr: &str) -> PyResult<()> {
        let v = with(|py| {
            let m = py.import(module)?;
            match m.getattr(attr) {
                Ok(v) => Ok(own(v)),
                // `from pkg import submodule`
                Err(_) => py.import(format!("{module}.{attr}").as_str()).map(|s| own(s.into_any())),
            }
        })
        .map_err(|e| if e.kind == "AttributeError" { Exc::new("ImportError", format!("cannot import name '{attr}' from '{module}'")) } else { e })?;
        *self.cell.lock().unwrap() = Some(v);
        Ok(())
    }

    pub fn get(&self) -> PyResult<Obj> {
        match &*self.cell.lock().unwrap() {
            Some(o) => Ok(o.clone()),
            None => Err(Exc::new("NameError", format!("name '{}' is not defined", self.binding))),
        }
    }
}

impl crate::conv::PyFmt for Obj {
    fn py_repr(&self) -> Str {
        Python::attach(|py| self.bind(py).repr().map(|s| Str::from(s.to_string())).unwrap_or_else(|_| Str::from("<repr failed>")))
    }
    fn py_str(&self) -> Str {
        Python::attach(|py| self.bind(py).str().map(|s| Str::from(s.to_string())).unwrap_or_else(|_| Str::from("<str failed>")))
    }
}

impl crate::Truthy for Obj {
    fn truthy(&self) -> bool {
        Python::attach(|py| self.bind(py).is_truthy().unwrap_or(true))
    }
}

/// Native value -> Python object (copies for immutable values; mutable
/// native collections never reach here: escape analysis keeps them boxed).
pub trait ToPy {
    fn to_py(&self) -> PyResult<Obj>;
}

/// Box a native value as a Python object.
pub fn to_py<T: ToPy + ?Sized>(v: &T) -> PyResult<Obj> {
    v.to_py()
}

impl ToPy for Obj {
    fn to_py(&self) -> PyResult<Obj> {
        Ok(self.clone())
    }
}
impl ToPy for i64 {
    fn to_py(&self) -> PyResult<Obj> {
        Ok(int(*self))
    }
}
impl ToPy for f64 {
    fn to_py(&self) -> PyResult<Obj> {
        Ok(float(*self))
    }
}
impl ToPy for bool {
    fn to_py(&self) -> PyResult<Obj> {
        Ok(bool_(*self))
    }
}
impl ToPy for Str {
    fn to_py(&self) -> PyResult<Obj> {
        Ok(str_(self))
    }
}
impl ToPy for () {
    fn to_py(&self) -> PyResult<Obj> {
        Ok(none())
    }
}
impl<T: ToPy> ToPy for Option<T> {
    fn to_py(&self) -> PyResult<Obj> {
        match self {
            Some(v) => v.to_py(),
            None => Ok(none()),
        }
    }
}
impl ToPy for Exc {
    fn to_py(&self) -> PyResult<Obj> {
        exc_obj(self)
    }
}
