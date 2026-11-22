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
    /// `tuple[T, ...]`: immutable, variable length (`rt::Tup<T>`).
    TupleVar(Ty),
    /// The element type of a container that never receives an element.
    Never,
    /// A live dict view: `d.keys()`, `d.values()`, `d.items()`.
    View(ViewKind, Ty, Ty),
    /// A native class object itself (`cls` in a classmethod, `Point` as a value).
    Type(ClassId, &'static str),
    /// A closed union of native class instances (`Square | Circle`), a Rust enum.
    Union(Vec<Ty>),
    /// A function value: which function (its id), the types of its captured
    /// variables, and its display name. Calls dispatch statically.
    Closure(u32, Vec<Ty>, &'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewKind {
    Keys,
    Values,
    Items,
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
                if ts.is_empty() {
                    "rt::Tuple0".to_string()
                } else if ts.len() == 1 {
                    format!("({},)", ts[0].rust())
                } else {
                    format!("({})", join(ts, Ty::rust, ", "))
                },
                format!("tup{}_{}", ts.len(), join(ts, Ty::mangle, "_")),
            ),
            Comp::Class(_, rust) => (rust.to_string(), rust.to_string(), rust.to_string()),
            Comp::Type(_, rust) => (format!("type[{rust}]"), "()".to_string(), format!("type_{rust}")),
            Comp::Union(ts) => (join(ts, Ty::py_name, " | "), format!("U_{}", join(ts, Ty::mangle, "_")), format!("u_{}", join(ts, Ty::mangle, "_"))),
            Comp::Closure(f, caps, name) => (
                format!("function {name}"),
                if caps.is_empty() { "rt::Env<()>".to_string() } else { format!("rt::Env<({},)>", caps.iter().map(|c| format!("rt::Cell<{}>", c.rust())).collect::<Vec<_>>().join(", ")) },
                format!("fn{f}_{}", join(caps, Ty::mangle, "_")),
            ),
            Comp::Func(ps, r) => (
                format!("Callable[[{}], {}]", join(ps, Ty::py_name, ", "), r.py_name()),
                format!("rt::Func<({}), {}>", ps.iter().map(|p| format!("{},", p.rust())).collect::<String>(), r.rust()),
                format!("fn{}_{}_{}", ps.len(), join(ps, Ty::mangle, "_"), r.mangle()),
            ),
            Comp::Opt(t) => (format!("{} | None", t.py_name()), format!("Option<{}>", t.rust()), format!("opt_{}", t.mangle())),
            Comp::Exc => ("BaseException".to_string(), "rt::Exc".to_string(), "exc".to_string()),
            Comp::TupleVar(t) => (format!("tuple[{}, ...]", t.py_name()), format!("rt::Tup<{}>", t.rust()), format!("tupv_{}", t.mangle())),
            Comp::Never => ("Never".to_string(), "rt::Never".to_string(), "never".to_string()),
            Comp::View(kind, k, v) => {
                let (py, rs) = match kind {
                    ViewKind::Keys => ("dict_keys", "Keys"),
                    ViewKind::Values => ("dict_values", "Values"),
                    ViewKind::Items => ("dict_items", "Items"),
                };
                (format!("{py}[{}, {}]", k.py_name(), v.py_name()), format!("rt::coll::{rs}<{}, {}>", k.rust(), v.rust()), format!("{}_{}_{}", rs.to_lowercase(), k.mangle(), v.mangle()))
            }
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

    /// A union of native class instance types (flattened, sorted, deduplicated).
    pub fn union(ts: Vec<Ty>) -> Ty {
        let mut all: Vec<Ty> = Vec::new();
        for t in ts {
            match t.as_comp() {
                Some(Comp::Union(inner)) => all.extend(inner),
                _ => all.push(t),
            }
        }
        all.sort_by_key(|t| (t.class_id(), t.mangle()));
        all.dedup();
        if all.len() == 1 {
            return all[0];
        }
        Ty::comp(Comp::Union(all))
    }

    pub fn closure(func: u32, caps: Vec<Ty>, name: &str) -> Ty {
        Ty::comp(Comp::Closure(func, caps, Box::leak(name.to_string().into_boxed_str())))
    }

    /// (function, captured variable types) of a function value.
    pub fn as_closure(self) -> Option<(usize, Vec<Ty>)> {
        match self.as_comp() {
            Some(Comp::Closure(f, caps, _)) => Some((f as usize, caps)),
            _ => None,
        }
    }

    /// Enum variant name of a union member (the class name, or the mangled type).
    pub fn variant_name(self) -> String {
        match self.class_id() {
            Some(_) => self.rust().to_string(),
            None => format!("V_{}", self.mangle()),
        }
    }

    pub fn union_members(self) -> Option<Vec<Ty>> {
        match self.as_comp() {
            Some(Comp::Union(ts)) => Some(ts),
            _ => None,
        }
    }

    /// Every union type created so far (codegen emits an enum for each).
    pub fn all_unions() -> Vec<Ty> {
        let i = interner().lock().unwrap();
        i.entries.iter().enumerate().filter(|(_, e)| matches!(e.comp, Comp::Union(_))).map(|(k, _)| Ty::C(TyId(k as u32))).collect()
    }

    pub fn never() -> Ty {
        Ty::comp(Comp::Never)
    }

    pub fn tuple_var(t: Ty) -> Ty {
        Ty::comp(Comp::TupleVar(t))
    }

    /// Element type when iterating a value of this type (`for x in v`).
    pub fn iter_elem(self) -> Option<Ty> {
        match self.as_comp()? {
            Comp::List(e) | Comp::Set(e) | Comp::TupleVar(e) => Some(e),
            Comp::Dict(k, _) | Comp::View(ViewKind::Keys, k, _) => Some(k),
            Comp::View(ViewKind::Values, _, v) => Some(v),
            Comp::View(ViewKind::Items, k, v) => Some(Ty::tuple(vec![k, v])),
            Comp::Tuple(ts) => {
                let Some(&first) = ts.first() else { return Some(Ty::never()) };
                ts.iter().all(|t| *t == first).then_some(first)
            }
            _ => None,
        }
    }

    pub fn is_never(self) -> bool {
        matches!(self.as_comp(), Some(Comp::Never))
    }

    /// Whether a native value of this type contains a mutable container
    /// anywhere (and so has identity that a copy would break).
    pub fn has_mutable(self) -> bool {
        match self.as_comp() {
            None => false,
            Some(Comp::List(_) | Comp::Dict(..) | Comp::Set(_) | Comp::Class(..)) => true,
            Some(Comp::Tuple(ts)) => ts.iter().any(|t| t.has_mutable()),
            Some(Comp::TupleVar(t) | Comp::Opt(t)) => t.has_mutable(),
            Some(Comp::View(..) | Comp::Union(_)) => true,
            _ => false,
        }
    }

    /// Valid native dict key / set element (hashable with Python equality).
    pub fn is_hashable(self) -> bool {
        match self {
            Ty::Int | Ty::Str | Ty::Bool | Ty::None => true,
            Ty::C(_) => match self.as_comp() {
                Some(Comp::Tuple(ts)) => ts.iter().all(|t| t.is_hashable()),
                Some(Comp::TupleVar(t) | Comp::Opt(t)) => t.is_hashable(),
                Some(Comp::Never) => true,
                Some(Comp::Class(..) | Comp::Union(_)) => true, // identity (or __hash__)
                _ => false,
            },
            _ => false,
        }
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

    /// An instance of native class `id` (`rust` is its generated type name).
    pub fn class(id: ClassId, rust: &str) -> Ty {
        Ty::comp(Comp::Class(id, Box::leak(rust.to_string().into_boxed_str())))
    }

    pub fn class_type(id: ClassId, rust: &str) -> Ty {
        Ty::comp(Comp::Type(id, Box::leak(rust.to_string().into_boxed_str())))
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

    /// Whether values of this type involve a CPython object anywhere.
    pub fn mentions_obj(self) -> bool {
        match self {
            Ty::Obj => true,
            Ty::C(_) => match self.as_comp() {
                Some(Comp::List(t) | Comp::Set(t) | Comp::Opt(t) | Comp::TupleVar(t)) => t.mentions_obj(),
                Some(Comp::Dict(k, v) | Comp::View(_, k, v)) => k.mentions_obj() || v.mentions_obj(),
                Some(Comp::Tuple(ts)) => ts.iter().any(|t| t.mentions_obj()),
                Some(Comp::Func(ps, r)) => r.mentions_obj() || ps.iter().any(|t| t.mentions_obj()),
                _ => false,
            },
            _ => false,
        }
    }

    /// Mutable, identity-carrying values (Python aliasing must be preserved).
    pub fn is_mutable(self) -> bool {
        matches!(self.as_comp(), Some(Comp::List(_) | Comp::Dict(..) | Comp::Set(_) | Comp::Class(..) | Comp::Union(_)))
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
