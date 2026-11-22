//! Concrete IR types (design section 6).
//!
//! `Int` is the documented fixed-width subset: Python `int` restricted to
//! i64, with checked arithmetic that raises `OverflowError` instead of
//! wrapping. Arbitrary precision promotion is milestone M4.
//!
//! Scalars are plain variants. Composite types (`list[T]`, `dict[K, V]`,
//! tuples, sets, class instances, functions, `Optional[T]`) are interned:
//! `Ty::C(id)` indexes a process-wide table of [`Comp`] values, so `Ty`
//! stays `Copy` and cheap to hash (it keys specializations). Interned
//! names are leaked once per distinct type, which keeps `py_name`,
//! `rust` and `mangle` returning `&'static str`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ty {
    Bool,
    Int,
    Float,
    Str,
    None,
    /// A boxed CPython object (`rt::py::Obj`), used at Python boundaries and
    /// wherever hybrid mode boxes a value whose type is not statically known.
    Obj,
    /// An interned composite type; see [`Comp`].
    C(TyId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TyId(u32);

/// Identifier of a natively compiled class (index into `Module::classes`).
pub type ClassId = u32;

/// Composite types.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Comp {
    /// `list[T]`: a shared, mutable vector (`rt::List<T>`).
    List(Ty),
    /// `dict[K, V]`: insertion-ordered (`rt::Dict<K, V>`).
    Dict(Ty, Ty),
    /// `set[T]` (`rt::Set<T>`).
    Set(Ty),
    /// Fixed-size `tuple[A, B, ...]`: a Rust tuple (value semantics).
    Tuple(Vec<Ty>),
    /// Instance of a natively compiled class (`rt::Ref<ClassName>`).
    Class(ClassId, &'static str),
    /// A function value: parameter and return types.
    Func(Vec<Ty>, Ty),
    /// `T | None` (`Option<T>`).
    Opt(Ty),
    /// An exception instance of a builtin exception type (`rt::Exc`).
    Exc,
}

struct Entry {
    comp: Comp,
    py: &'static str,
    rust: &'static str,
    mangle: &'static str,
}

#[derive(Default)]
struct Interner {
    entries: Vec<Entry>,
    index: HashMap<Comp, u32>,
}

fn interner() -> &'static Mutex<Interner> {
    static I: OnceLock<Mutex<Interner>> = OnceLock::new();
    I.get_or_init(|| Mutex::new(Interner::default()))
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn join<F: Fn(Ty) -> &'static str>(ts: &[Ty], f: F, sep: &str) -> String {
    ts.iter().map(|t| f(*t)).collect::<Vec<_>>().join(sep)
}

impl Ty {
    /// Intern a composite type.
    pub fn comp(c: Comp) -> Ty {
        // Names are computed outside the lock: they recurse into `py_name` etc.
        if let Some(&id) = interner().lock().unwrap().index.get(&c) {
            return Ty::C(TyId(id));
        }
        let (py, rust, mangle) = match &c {
            Comp::List(e) => (format!("list[{}]", e.py_name()), format!("rt::List<{}>", e.rust()), format!("list_{}", e.mangle())),
            Comp::Dict(k, v) => (
                format!("dict[{}, {}]", k.py_name(), v.py_name()),
                format!("rt::Dict<{}, {}>", k.rust(), v.rust()),
                format!("dict_{}_{}", k.mangle(), v.mangle()),
            ),
            Comp::Set(e) => (format!("set[{}]", e.py_name()), format!("rt::Set<{}>", e.rust()), format!("set_{}", e.mangle())),
            Comp::Tuple(ts) => (
                if ts.is_empty() { "tuple[()]".to_string() } else { format!("tuple[{}]", join(ts, Ty::py_name, ", ")) },
                if ts.len() == 1 { format!("({},)", ts[0].rust()) } else { format!("({})", join(ts, Ty::rust, ", ")) },
                format!("tup{}_{}", ts.len(), join(ts, Ty::mangle, "_")),
            ),
            Comp::Class(_, name) => (name.to_string(), format!("rt::Ref<{}>", name.replace('.', "_")), name.replace('.', "_")),
            Comp::Func(ps, r) => (
                format!("Callable[[{}], {}]", join(ps, Ty::py_name, ", "), r.py_name()),
                format!("rt::Func<({}), {}>", ps.iter().map(|p| format!("{},", p.rust())).collect::<String>(), r.rust()),
                format!("fn{}_{}_{}", ps.len(), join(ps, Ty::mangle, "_"), r.mangle()),
            ),
            Comp::Opt(t) => (format!("{} | None", t.py_name()), format!("Option<{}>", t.rust()), format!("opt_{}", t.mangle())),
            Comp::Exc => ("BaseException".to_string(), "rt::Exc".to_string(), "exc".to_string()),
        };
        let mut i = interner().lock().unwrap();
        if let Some(&id) = i.index.get(&c) {
            return Ty::C(TyId(id));
        }
        let id = i.entries.len() as u32;
        i.entries.push(Entry { comp: c.clone(), py: leak(py), rust: leak(rust), mangle: leak(mangle) });
        i.index.insert(c, id);
        Ty::C(TyId(id))
    }

    pub fn list(elem: Ty) -> Ty {
        Ty::comp(Comp::List(elem))
    }

    pub fn dict(k: Ty, v: Ty) -> Ty {
        Ty::comp(Comp::Dict(k, v))
    }

    pub fn set(elem: Ty) -> Ty {
        Ty::comp(Comp::Set(elem))
    }

    pub fn tuple(items: Vec<Ty>) -> Ty {
        Ty::comp(Comp::Tuple(items))
    }

    /// A caught or constructed exception.
    pub fn exc() -> Ty {
        Ty::comp(Comp::Exc)
    }

    pub fn opt(t: Ty) -> Ty {
        if t == Ty::Obj || t == Ty::None || t.as_opt().is_some() {
            return t;
        }
        Ty::comp(Comp::Opt(t))
    }

    /// The composite behind `C(id)`, if any.
    pub fn as_comp(self) -> Option<Comp> {
        match self {
            Ty::C(TyId(id)) => Some(interner().lock().unwrap().entries[id as usize].comp.clone()),
            _ => None,
        }
    }

    pub fn list_elem(self) -> Option<Ty> {
        match self.as_comp() {
            Some(Comp::List(e)) => Some(e),
            _ => None,
        }
    }

    pub fn dict_kv(self) -> Option<(Ty, Ty)> {
        match self.as_comp() {
            Some(Comp::Dict(k, v)) => Some((k, v)),
            _ => None,
        }
    }

    pub fn set_elem(self) -> Option<Ty> {
        match self.as_comp() {
            Some(Comp::Set(e)) => Some(e),
            _ => None,
        }
    }

    pub fn tuple_items(self) -> Option<Vec<Ty>> {
        match self.as_comp() {
            Some(Comp::Tuple(ts)) => Some(ts),
            _ => None,
        }
    }

    pub fn class_id(self) -> Option<ClassId> {
        match self.as_comp() {
            Some(Comp::Class(id, _)) => Some(id),
            _ => None,
        }
    }

    pub fn as_func(self) -> Option<(Vec<Ty>, Ty)> {
        match self.as_comp() {
            Some(Comp::Func(ps, r)) => Some((ps, r)),
            _ => None,
        }
    }

    pub fn as_opt(self) -> Option<Ty> {
        match self.as_comp() {
            Some(Comp::Opt(t)) => Some(t),
            _ => None,
        }
    }

    /// Mutable, identity-carrying values (Python aliasing must be preserved).
    pub fn is_mutable(self) -> bool {
        matches!(self.as_comp(), Some(Comp::List(_) | Comp::Dict(..) | Comp::Set(_) | Comp::Class(..)))
    }

    fn entry<R>(self, f: impl FnOnce(&Entry) -> R) -> R {
        let Ty::C(TyId(id)) = self else { unreachable!() };
        f(&interner().lock().unwrap().entries[id as usize])
    }

    pub fn py_name(self) -> &'static str {
        match self {
            Ty::Bool => "bool",
            Ty::Int => "int",
            Ty::Float => "float",
            Ty::Str => "str",
            Ty::None => "None",
            Ty::Obj => "object",
            Ty::C(_) => self.entry(|e| e.py),
        }
    }

    pub fn rust(self) -> &'static str {
        match self {
            Ty::Bool => "bool",
            Ty::Int => "i64",
            Ty::Float => "f64",
            Ty::Str => "rt::Str",
            Ty::None => "()",
            Ty::Obj => "rt::py::Obj",
            Ty::C(_) => self.entry(|e| e.rust),
        }
    }

    /// Short mangling component for specialized function names.
    pub fn mangle(self) -> &'static str {
        match self {
            Ty::Bool => "bool",
            Ty::Int => "int",
            Ty::Float => "float",
            Ty::Str => "str",
            Ty::None => "none",
            Ty::Obj => "obj",
            Ty::C(_) => self.entry(|e| e.mangle),
        }
    }

    pub fn is_numeric(self) -> bool {
        matches!(self, Ty::Bool | Ty::Int | Ty::Float)
    }

    /// Zero-cost Rust initializer for a declared-but-not-yet-assigned slot.
    /// Only used for locals proven definitely-assigned before every read.
    pub fn rust_placeholder(self) -> &'static str {
        match self {
            Ty::Bool => "false",
            Ty::Int => "0",
            Ty::Float => "0.0",
            Ty::Str => "rt::Str::from(\"\")",
            Ty::None => "()",
            Ty::Obj => "rt::py::none()",
            Ty::C(_) => "Default::default()",
        }
    }

    /// Values that are cloned (not copied) when read.
    pub fn is_clone(self) -> bool {
        matches!(self, Ty::Str | Ty::Obj | Ty::C(_))
    }
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.py_name())
    }
}

impl serde::Serialize for Ty {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // Scalars keep their historical lowercase spelling (`none`).
        s.serialize_str(match self {
            Ty::None => "none",
            t => t.py_name(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_is_stable() {
        let a = Ty::list(Ty::Int);
        assert_eq!(a, Ty::list(Ty::Int));
        assert_ne!(a, Ty::list(Ty::Str));
        assert_eq!(a.py_name(), "list[int]");
        assert_eq!(a.rust(), "rt::List<i64>");
        assert_eq!(a.list_elem(), Some(Ty::Int));
        let d = Ty::dict(Ty::Str, a);
        assert_eq!(d.py_name(), "dict[str, list[int]]");
        assert_eq!(d.mangle(), "dict_str_list_int");
        let t = Ty::tuple(vec![Ty::Int, Ty::Str]);
        assert_eq!(t.rust(), "(i64, rt::Str)");
        assert_eq!(Ty::tuple(vec![Ty::Int]).rust(), "(i64,)");
        assert_eq!(Ty::opt(Ty::Int).py_name(), "int | None");
        assert_eq!(Ty::opt(Ty::opt(Ty::Int)), Ty::opt(Ty::Int));
        assert!(a.is_mutable() && !t.is_mutable());
    }
}
