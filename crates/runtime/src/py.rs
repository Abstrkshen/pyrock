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
    let r = exit_call(cm, "__exit__", err)?;
    with(|py| r.bind(py).is_truthy())
}

/// `type(cm).__aenter__(cm)` (the awaitable).
pub fn aenter_call(cm: &Obj) -> PyResult<Obj> {
    with(|py| {
        let b = cm.bind(py);
        call_special(b, "__aenter__", vec![]).map(own)
    })
}

/// Call `__exit__` / `__aexit__` with the exception leaving the body.
pub fn exit_call(cm: &Obj, method: &str, err: Option<&Exc>) -> PyResult<Obj> {
    let exc = match err {
        Some(e) => Some(exc_obj(e)?),
        None => None,
    };
    with(|py| {
        let b = cm.bind(py);
        let r = match exc {
            None => call_special(b, method, vec![py.None().into_bound(py), py.None().into_bound(py), py.None().into_bound(py)])?,
            Some(x) => {
                let v = x.bind(py);
                call_special(b, method, vec![v.get_type().into_any(), v.clone(), v.getattr("__traceback__")?])?
            }
        };
        Ok(own(r))
    })
}

/// `bool(o)` for a Python object.
pub fn is_true(o: &Obj) -> PyResult<bool> {
    with(|py| o.bind(py).is_truthy())
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
        // The build interpreter's `sys.path` (its stdlib, its virtualenv's
        // site-packages, the source folder), unless PYROCK_SYS_PATH replaces
        // it: the binary then runs with another installation of the same
        // Python version.
        let over = std::env::var("PYROCK_SYS_PATH").ok();
        let sep = if cfg!(windows) { ';' } else { ':' };
        let paths: Vec<&str> = match &over {
            Some(v) => v.split(sep).filter(|p| !p.is_empty()).collect(),
            None => paths.to_vec(),
        };
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
        eprintln!("pyrock: failed to initialise embedded CPython: {e}");
        std::process::exit(1);
    }
}

/// Run the program attached to the interpreter (holding the GIL): every
/// bridge operation then only bumps PyO3's attach counter, and dropped
/// objects are released at once instead of being queued for later. Python
/// code that blocks (I/O, `time.sleep`, the asyncio selector) releases the
/// GIL itself, as in CPython.
pub fn attached<R>(f: impl FnOnce() -> R) -> R {
    Python::attach(|_| f())
}

// ---- boxing native values ----

/// A Python string constant, created (interned) on first use.
pub struct Interned {
    text: &'static str,
    cell: std::sync::OnceLock<Py<PyAny>>,
}

impl Interned {
    pub const fn new(text: &'static str) -> Interned {
        Interned { text, cell: std::sync::OnceLock::new() }
    }

    #[inline]
    pub fn get(&self) -> Obj {
        Python::attach(|py| Obj(self.cell.get_or_init(|| pyo3::types::PyString::intern(py, self.text).clone().into_any().unbind()).clone_ref(py)))
    }
}

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
                "annotation `{ann}` expects {ann}, got {got} (pyrock enforces annotations where it unboxes Python objects)"
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
                Err(_) => match py.import(format!("{module}.{attr}").as_str()) {
                    Ok(s) => Ok(own(s.into_any())),
                    // CPython's message: the module's file, if it has one.
                    Err(e) if e.is_instance_of::<pyo3::exceptions::PyModuleNotFoundError>(py) => {
                        let loc = m
                            .getattr("__file__")
                            .ok()
                            .filter(|f| !f.is_none())
                            .and_then(|f| f.str().ok().map(|s| s.to_string()))
                            .unwrap_or_else(|| "unknown location".to_string());
                        Err(pyo3::exceptions::PyImportError::new_err(format!("cannot import name '{attr}' from '{module}' ({loc})")))
                    }
                    Err(e) => Err(e),
                },
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

// ---- collections <-> Python (copies: escape analysis keeps shared
// mutable containers that reach Python boxed from creation) ----

impl<T: ToPy> ToPy for crate::List<T>
where
    T: Clone,
{
    fn to_py(&self) -> PyResult<Obj> {
        let items: Vec<Obj> = self.to_vec().iter().map(|v| v.to_py()).collect::<PyResult<_>>()?;
        list(items)
    }
}

impl<T: ToPy + Clone> ToPy for crate::Tup<T> {
    fn to_py(&self) -> PyResult<Obj> {
        let items: Vec<Obj> = self.to_vec().iter().map(|v| v.to_py()).collect::<PyResult<_>>()?;
        tuple(items)
    }
}

impl ToPy for crate::Tuple0 {
    fn to_py(&self) -> PyResult<Obj> {
        tuple(vec![])
    }
}

impl ToPy for crate::Never {
    fn to_py(&self) -> PyResult<Obj> {
        unreachable!("a value of an empty container's element type")
    }
}

impl<K: ToPy + Clone + Eq + std::hash::Hash, V: ToPy + Clone> ToPy for crate::Dict<K, V> {
    fn to_py(&self) -> PyResult<Obj> {
        let mut items = Vec::new();
        for (k, v) in self.items_vec() {
            items.push((k.to_py()?, v.to_py()?));
        }
        dict(items)
    }
}

impl<T: ToPy + Clone + Eq + std::hash::Hash> ToPy for crate::Set<T> {
    fn to_py(&self) -> PyResult<Obj> {
        let items: Vec<Obj> = self.to_vec().iter().map(|v| v.to_py()).collect::<PyResult<_>>()?;
        let l = list(items)?;
        call(&builtin("set")?, vec![l], vec![])
    }
}

macro_rules! tuple_to_py {
    ($($n:tt $T:ident),+) => {
        impl<$($T: ToPy),+> ToPy for ($($T,)+) {
            fn to_py(&self) -> PyResult<Obj> {
                tuple(vec![$(self.$n.to_py()?),+])
            }
        }
    };
}
tuple_to_py!(0 A);
tuple_to_py!(0 A, 1 B);
tuple_to_py!(0 A, 1 B, 2 C);
tuple_to_py!(0 A, 1 B, 2 C, 3 D);
tuple_to_py!(0 A, 1 B, 2 C, 3 D, 4 E);
tuple_to_py!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F);
tuple_to_py!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G);
tuple_to_py!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H);
tuple_to_py!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H, 8 I);
tuple_to_py!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H, 8 I, 9 J);
tuple_to_py!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H, 8 I, 9 J, 10 K);
tuple_to_py!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H, 8 I, 9 J, 10 K, 11 L);

impl crate::coll::PyEq for Obj {
    fn py_eq(&self, o: &Self) -> bool {
        Python::attach(|py| self.bind(py).eq(o.bind(py)).unwrap_or(false))
    }
}

impl crate::coll::PyOrd for Obj {
    fn py_cmp(&self, o: &Self) -> PyResult<std::cmp::Ordering> {
        with(|py| {
            let (a, b) = (self.bind(py), o.bind(py));
            if a.lt(b)? {
                Ok(std::cmp::Ordering::Less)
            } else if b.lt(a)? {
                Ok(std::cmp::Ordering::Greater)
            } else {
                Ok(std::cmp::Ordering::Equal)
            }
        })
    }
}

/// Elements of a Python iterable (for `list(obj)`, `sorted(obj)`, ...).
pub fn to_vec(o: &Obj) -> PyResult<Vec<Obj>> {
    let mut it = iter(o)?;
    let mut out = Vec::new();
    while let Some(v) = it.next()? {
        out.push(v);
    }
    Ok(out)
}

/// Unpack exactly `n` items from a Python iterable (`a, b = obj`).
pub fn unpack(o: &Obj, n: usize) -> PyResult<Vec<Obj>> {
    let v = to_vec(o)?;
    if v.len() != n {
        let msg = if v.len() > n {
            format!("too many values to unpack (expected {n})")
        } else {
            format!("not enough values to unpack (expected {n}, got {})", v.len())
        };
        return Err(Exc::new("ValueError", msg));
    }
    Ok(v)
}

/// `slice(lo, hi, step)` for a Python receiver.
pub fn slice(lo: Obj, hi: Obj, step: Obj) -> PyResult<Obj> {
    with(|py| Ok(own(pyo3::types::PySlice::full(py).get_type().call1((lo.0, hi.0, step.0))?)))
}

/// A Python object used as a slice bound of a native sequence (`None` or an int).
pub fn opt_index(o: &Obj) -> PyResult<Option<i64>> {
    with(|py| {
        let b = o.bind(py);
        if b.is_none() {
            Ok(None)
        } else {
            b.extract::<i64>().map(Some)
        }
    })
}

pub fn delitem(o: &Obj, k: Obj) -> PyResult<()> {
    with(|py| o.bind(py).del_item(k.0))
}

pub fn delattr(o: &Obj, name: &str) -> PyResult<()> {
    with(|py| o.bind(py).delattr(name))
}

macro_rules! view_to_py {
    ($name:ident) => {
        impl<K: ToPy + Clone + Eq + std::hash::Hash, V: ToPy + Clone> ToPy for crate::coll::$name<K, V> {
            fn to_py(&self) -> PyResult<Obj> {
                let d = self.0.to_py()?;
                let m = match stringify!($name) {
                    "Keys" => "keys",
                    "Values" => "values",
                    _ => "items",
                };
                call(&getattr(&d, m)?, vec![], vec![])
            }
        }
    };
}
view_to_py!(Keys);
view_to_py!(Values);
view_to_py!(Items);

/// `format(obj, spec)` through CPython.
pub fn format(o: &Obj, spec: &str) -> PyResult<Str> {
    with(|py| {
        let r = py.import("builtins")?.getattr("format")?.call1((o.bind(py), spec))?;
        Ok(Str::from(r.extract::<String>()?))
    })
}

/// `f(*args, **kwargs)` with keyword names known only at run time.
pub fn call_dyn(f: &Obj, args: Vec<Obj>, kwargs: Vec<(String, Obj)>) -> PyResult<Obj> {
    with(|py| {
        let args = PyTuple::new(py, args.into_iter().map(|a| a.0))?;
        let kw = PyDict::new(py);
        for (k, v) in kwargs {
            if kw.contains(&k)? {
                return Err(pyo3::exceptions::PyTypeError::new_err(format!("got multiple values for keyword argument '{k}'")));
            }
            kw.set_item(k, v.0)?;
        }
        f.bind(py).call(args, Some(&kw)).map(own)
    })
}

/// The items of a `**mapping` argument.
pub fn kw_items(o: &Obj) -> PyResult<Vec<(String, Obj)>> {
    with(|py| {
        let b = o.bind(py);
        let mut out = Vec::new();
        for k in b.call_method0("keys")?.try_iter()? {
            let k = k?;
            let v = b.get_item(&k)?;
            out.push((k.extract::<String>()?, own(v)));
        }
        Ok(out)
    })
}

// ---- native functions callable from Python ----

/// A native function value exposed to CPython (callbacks, decorators,
/// `RunnableLambda(lambda x: ...)`). Single-threaded: CPython calls it while
/// holding the GIL on the thread that created it.
#[pyclass(name = "native_function")]
pub struct NativeFn {
    name: String,
    #[allow(clippy::type_complexity)]
    f: Box<dyn Fn(Vec<Obj>, Vec<(String, Obj)>) -> PyResult<Obj>>,
}

/// A native exception as a `PyErr` (the original object if it came from Python).
pub fn to_pyerr(e: Exc) -> PyErr {
    match exc_obj(&e) {
        Ok(o) => Python::attach(|py| PyErr::from_value(o.bind(py).clone())),
        Err(_) => pyo3::exceptions::PyRuntimeError::new_err(e.to_string()),
    }
}

#[pymethods]
impl NativeFn {
    #[pyo3(signature = (*args, **kwargs))]
    fn __call__(&self, args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>) -> pyo3::PyResult<Py<PyAny>> {
        let a: Vec<Obj> = args.iter().map(own).collect();
        let mut k = Vec::new();
        if let Some(d) = kwargs {
            for (key, v) in d.iter() {
                k.push((key.extract::<String>()?, own(v)));
            }
        }
        match (self.f)(a, k) {
            Ok(o) => Ok(o.0),
            Err(e) => Err(to_pyerr(e)),
        }
    }

    fn __repr__(&self) -> String {
        format!("<native function {}>", self.name)
    }

    #[getter]
    fn __name__(&self) -> String {
        self.name.clone()
    }
}

/// Wrap a native function value as a Python callable.
pub fn native_fn(name: &str, f: impl Fn(Vec<Obj>, Vec<(String, Obj)>) -> PyResult<Obj> + 'static) -> Obj {
    Python::attach(|py| {
        let o = Py::new(py, NativeFn { name: name.to_string(), f: Box::new(f) }).expect("native function object");
        Obj(o.into_any())
    })
}

/// Bind a Python call's arguments to `params` (positional then keyword);
/// `None` marks a parameter left to its default.
pub fn bind_py_args(fname: &str, params: &[&str], args: Vec<Obj>, kwargs: Vec<(String, Obj)>) -> PyResult<Vec<Option<Obj>>> {
    if args.len() > params.len() {
        return Err(Exc::new("TypeError", format!("{fname}() takes {} positional argument(s) but {} were given", params.len(), args.len())));
    }
    let mut out: Vec<Option<Obj>> = vec![None; params.len()];
    for (i, a) in args.into_iter().enumerate() {
        out[i] = Some(a);
    }
    for (k, v) in kwargs {
        let Some(i) = params.iter().position(|p| *p == k) else {
            return Err(Exc::new("TypeError", format!("{fname}() got an unexpected keyword argument '{k}'")));
        };
        if out[i].is_some() {
            return Err(Exc::new("TypeError", format!("{fname}() got multiple values for argument '{k}'")));
        }
        out[i] = Some(v);
    }
    Ok(out)
}

/// A missing required argument (CPython's TypeError).
pub fn missing_arg(fname: &str, param: &str) -> Exc {
    Exc::new("TypeError", format!("{fname}() missing 1 required positional argument: '{param}'"))
}

// ---- native class instances seen from Python ----

/// What CPython can do with a native class instance (generated per class).
pub trait NativeObject: 'static {
    fn class_name(&self) -> &'static str;
    /// Attribute read: a field (boxed), a property, or a bound method.
    fn get(&self, name: &str) -> PyResult<Obj>;
    /// Attribute write (scalar fields).
    fn set(&self, name: &str, v: Obj) -> PyResult<()>;
    fn repr(&self) -> Str;
    fn str_(&self) -> Str;
    fn eq_obj(&self, other: &NativeObj) -> bool;
    fn hash(&self) -> i64;
    fn as_any(&self) -> &dyn std::any::Any;
}

#[pyclass(weakref, name = "native_object")]
pub struct NativeObj {
    pub inner: Box<dyn NativeObject>,
}

// SAFETY: native functions and objects may be called from any CPython thread
// (`threading`, thread pools): CPython calls them holding the GIL, which
// serialises every run of compiled code (see `rt::Shared`), so their `Rc`
// state is never touched by two threads at once.
unsafe impl Send for NativeFn {}
unsafe impl Sync for NativeFn {}
unsafe impl Send for NativeObj {}
unsafe impl Sync for NativeObj {}

#[pymethods]
impl NativeObj {
    fn __getattr__(&self, name: &str) -> pyo3::PyResult<Py<PyAny>> {
        self.inner.get(name).map(|o| o.0).map_err(to_pyerr)
    }

    fn __setattr__(&self, name: &str, v: Bound<'_, PyAny>) -> pyo3::PyResult<()> {
        self.inner.set(name, own(v)).map_err(to_pyerr)
    }

    fn __repr__(&self) -> String {
        self.inner.repr().to_string()
    }

    fn __str__(&self) -> String {
        self.inner.str_().to_string()
    }

    fn __eq__(&self, other: Bound<'_, PyAny>) -> bool {
        match other.cast::<NativeObj>() {
            Ok(o) => self.inner.eq_obj(&o.borrow()),
            Err(_) => false,
        }
    }

    fn __hash__(&self) -> i64 {
        self.inner.hash()
    }
}

/// The Python object for a native instance (cached: identity is preserved
/// while the Python side holds it).
pub fn native_obj<T: NativeObject + Clone>(v: &T, cache: &crate::PyCache) -> PyResult<Obj> {
    Python::attach(|py| {
        if let Some(w) = cache.borrow().as_ref() {
            if let Some(o) = w.bind(py).upgrade() {
                return Ok(own(o));
            }
        }
        let o = Py::new(py, NativeObj { inner: Box::new(v.clone()) }).map_err(|e| to_exc(py, e))?;
        let w = pyo3::types::PyWeakrefReference::new(o.bind(py).as_any()).map_err(|e| to_exc(py, e))?;
        *cache.borrow_mut() = Some(w.unbind());
        Ok(Obj(o.into_any()))
    })
}

/// A Python value back to a native scalar (attribute writes from Python).
pub trait FromPy: Sized {
    fn from_py(o: &Obj) -> PyResult<Self>;
}
impl FromPy for i64 {
    fn from_py(o: &Obj) -> PyResult<Self> {
        unbox_annotated::<i64>(o, "int")
    }
}
impl FromPy for bool {
    fn from_py(o: &Obj) -> PyResult<Self> {
        unbox_annotated::<bool>(o, "bool")
    }
}
impl FromPy for Str {
    fn from_py(o: &Obj) -> PyResult<Self> {
        unbox_annotated::<Str>(o, "str")
    }
}
impl FromPy for f64 {
    fn from_py(o: &Obj) -> PyResult<Self> {
        with(|py| {
            let b = o.bind(py);
            if b.is_instance_of::<pyo3::types::PyFloat>() {
                b.extract::<f64>()
            } else {
                Err(pyo3::exceptions::PyTypeError::new_err("expected a float"))
            }
        })
    }
}
impl FromPy for Obj {
    fn from_py(o: &Obj) -> PyResult<Self> {
        Ok(o.clone())
    }
}
impl<T: FromPy> FromPy for Option<T> {
    fn from_py(o: &Obj) -> PyResult<Self> {
        if Python::attach(|py| o.bind(py).is_none()) {
            Ok(None)
        } else {
            T::from_py(o).map(Some)
        }
    }
}

pub fn from_py<T: FromPy>(o: &Obj) -> PyResult<T> {
    T::from_py(o)
}

/// `o[i] = items[i]` for every item (write-back into a Python sequence such as
/// a numpy array or `array.array`).
pub fn set_all(o: &Obj, items: Vec<Obj>) -> PyResult<()> {
    with(|py| {
        // Item by item: `array.array` only accepts arrays for slice assignment.
        let b = o.bind(py);
        for (i, a) in items.into_iter().enumerate() {
            b.set_item(i, a.0)?;
        }
        Ok(())
    })
}

/// A Python `bytes` object.
pub fn bytes(v: &[u8]) -> Obj {
    Python::attach(|py| own(pyo3::types::PyBytes::new(py, v).into_any()))
}

impl Obj {
    /// Take ownership of a PyO3 object.
    pub fn from_py(o: Py<PyAny>) -> Obj {
        Obj(o)
    }

    /// The PyO3 object.
    pub fn into_py(self) -> Py<PyAny> {
        self.0
    }
}

/// Run Python source as a module and return one of its attributes.
pub fn run_module_attr(src: &std::ffi::CStr, file: &std::ffi::CStr, module: &std::ffi::CStr, attr: &str) -> PyResult<Obj> {
    with(|py| PyModule::from_code(py, src, file, module)?.getattr(attr).map(own))
}

/// Placeholder for not-yet-assigned locals captured by closures (`None`).
impl Default for Obj {
    fn default() -> Obj {
        none()
    }
}

/// `type(o).name(o, *args)` (special-method lookup, as CPython's protocols do).
pub fn call_method(o: &Obj, name: &str, args: Vec<Obj>) -> PyResult<Obj> {
    with(|py| {
        let b = o.bind(py);
        call_special(b, name, args.into_iter().map(|a| a.0.into_bound(py)).collect()).map(own)
    })
}

/// `type(o).name(o, *args)`, or `o.name(*args)` when the type has no such
/// attribute (a compiled class instance answers through `__getattr__`).
fn call_special<'py>(b: &Bound<'py, PyAny>, name: &str, args: Vec<Bound<'py, PyAny>>) -> pyo3::PyResult<Bound<'py, PyAny>> {
    let py = b.py();
    match b.get_type().getattr(name) {
        Ok(f) => {
            let mut all = vec![b.clone()];
            all.extend(args);
            f.call1(PyTuple::new(py, all)?)
        }
        Err(_) => b.getattr(name)?.call1(PyTuple::new(py, args)?),
    }
}

/// `o is None`
pub fn is_none(o: &Obj) -> bool {
    Python::attach(|py| o.bind(py).is_none())
}
