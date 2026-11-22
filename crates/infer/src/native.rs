//! Typing tables for native collections, strings and builtins.
//!
//! Inference, the boundary planner and codegen all consult these tables so
//! they cannot disagree about what runs natively. `None` from a lookup
//! means "not native": the operation goes through CPython (hybrid mode).

use pyfast_ir::{Comp, Ty};

/// Join two types for one storage location. Mutable containers join their
/// element types (`list[int] ⊔ list[str] = list[object]` when boxing), so a
/// container is never copied just because its elements disagree.
pub fn ty_join(a: Ty, b: Ty, boxing: bool) -> Option<Ty> {
    if a == b {
        return Some(a);
    }
    if a == Ty::Obj || b == Ty::Obj {
        return None;
    }
    if a.is_never() {
        return Some(b);
    }
    if b.is_never() {
        return Some(a);
    }
    match (a, b) {
        (Ty::None, x) | (x, Ty::None) => return Some(Ty::opt(x)),
        _ => {}
    }
    if let Some(x) = a.as_opt() {
        return ty_join(x, b, boxing).map(Ty::opt);
    }
    if let Some(y) = b.as_opt() {
        return ty_join(a, y, boxing).map(Ty::opt);
    }
    let elem = |x: Ty, y: Ty| ty_join(x, y, boxing).or(boxing.then_some(Ty::Obj));
    // Instances of different native classes: a closed union (a Rust enum).
    let classy = |t: Ty| t.class_id().is_some() || t.union_members().is_some();
    if classy(a) && classy(b) {
        return Some(Ty::union(vec![a, b]));
    }
    match (a.as_comp()?, b.as_comp()?) {
        (Comp::List(x), Comp::List(y)) => Some(Ty::list(elem(x, y)?)),
        (Comp::Set(x), Comp::Set(y)) => {
            let e = ty_join(x, y, boxing)?;
            e.is_hashable().then(|| Ty::set(e))
        }
        (Comp::Dict(k1, v1), Comp::Dict(k2, v2)) => {
            let k = ty_join(k1, k2, boxing)?;
            k.is_hashable().then_some(())?;
            Some(Ty::dict(k, elem(v1, v2)?))
        }
        (Comp::Tuple(xs), Comp::Tuple(ys)) if xs.len() == ys.len() => {
            let items: Option<Vec<Ty>> = xs.iter().zip(&ys).map(|(x, y)| elem(*x, *y)).collect();
            Some(Ty::tuple(items?))
        }
        (Comp::Tuple(xs), Comp::Tuple(ys)) => Some(Ty::tuple_var(join_all(xs.iter().chain(&ys).copied(), boxing)?)),
        (Comp::Tuple(xs), Comp::TupleVar(y)) | (Comp::TupleVar(y), Comp::Tuple(xs)) => {
            Some(Ty::tuple_var(join_all(xs.iter().copied().chain([y]), boxing)?))
        }
        (Comp::TupleVar(x), Comp::TupleVar(y)) => Some(Ty::tuple_var(elem(x, y)?)),
        _ => None,
    }
}

/// Join of many element types (empty → Never).
pub fn join_all(ts: impl IntoIterator<Item = Ty>, boxing: bool) -> Option<Ty> {
    let mut acc = Ty::never();
    for t in ts {
        acc = ty_join(acc, t, boxing).or(boxing.then_some(Ty::Obj))?;
    }
    Some(acc)
}

/// Element type of `for x in <t>` (and of builtins that consume iterables).
pub fn iter_elem(t: Ty) -> Option<Ty> {
    match t {
        Ty::Str => Some(Ty::Str),
        Ty::Obj => Some(Ty::Obj),
        t => t.iter_elem(),
    }
}

fn int_like(t: Ty) -> bool {
    matches!(t, Ty::Int | Ty::Bool)
}

fn base(t: Ty) -> Ty {
    t.as_opt().unwrap_or(t)
}

/// What a subscript `obj[index]` produces. `index_lit` is the index when it
/// is an int literal; `slice_lits` the literal bounds of a slice (if all are).
pub enum Index {
    Item,
    Slice,
}

pub fn subscript(obj: Ty, idx: Index, index_ty: Option<Ty>, index_lit: Option<i64>, slice_lits: Option<[Option<i64>; 3]>) -> Result<Ty, String> {
    let name = obj.py_name();
    match (obj.as_comp(), &idx) {
        _ if obj == Ty::Str => match idx {
            Index::Slice => Ok(Ty::Str),
            Index::Item if index_ty.map(base).is_some_and(int_like) => Ok(Ty::Str),
            Index::Item => Err(format!("string indices must be integers, not '{}'", index_ty.map(|t| t.py_name()).unwrap_or("?"))),
        },
        (Some(Comp::List(e)), Index::Item) => {
            if index_ty.map(base).is_some_and(int_like) {
                Ok(e)
            } else {
                Err(format!("list indices must be integers or slices, not {}", index_ty.map(|t| t.py_name()).unwrap_or("?")))
            }
        }
        (Some(Comp::List(_)), Index::Slice) => Ok(obj),
        (Some(Comp::TupleVar(e)), Index::Item) if index_ty.map(base).is_some_and(int_like) => Ok(e),
        (Some(Comp::TupleVar(_)), Index::Slice) => Ok(obj),
        (Some(Comp::Tuple(ts)), Index::Item) if index_ty.map(base).is_some_and(int_like) => match index_lit {
            Some(i) => {
                let n = ts.len() as i64;
                let j = if i < 0 { i + n } else { i };
                if (0..n).contains(&j) {
                    Ok(ts[j as usize])
                } else {
                    Err("tuple index out of range".into())
                }
            }
            None => {
                let first = ts.first().copied().unwrap_or(Ty::never());
                if ts.iter().all(|t| *t == first) {
                    Ok(first)
                } else {
                    Err(format!("indexing a heterogeneous {name} with a non-literal index has no single type"))
                }
            }
        },
        (Some(Comp::Tuple(ts)), Index::Slice) => match slice_lits {
            Some([lo, hi, st]) => {
                let pos = slice_positions(lo, hi, st, ts.len()).map_err(|e| e.to_string())?;
                Ok(Ty::tuple(pos.into_iter().map(|p| ts[p]).collect()))
            }
            None => {
                let first = ts.first().copied().unwrap_or(Ty::never());
                if ts.iter().all(|t| *t == first) {
                    Ok(Ty::tuple_var(first))
                } else {
                    Err(format!("slicing a heterogeneous {name} with non-literal bounds has no single type"))
                }
            }
        },
        (Some(Comp::Dict(k, v)), Index::Item) => match index_ty {
            Some(t) if ty_join(k, t, false) == Some(k) || k.is_never() => Ok(v),
            Some(t) => Err(format!("dict key of type {t} for a {name}")),
            None => Ok(v),
        },
        _ => Err(format!("'{name}' object is not subscriptable")),
    }
}

/// CPython's slice position arithmetic, at compile time (for tuple types).
fn slice_positions(lo: Option<i64>, hi: Option<i64>, step: Option<i64>, len: usize) -> Result<Vec<usize>, &'static str> {
    let n = len as i64;
    let step = step.unwrap_or(1);
    if step == 0 {
        return Err("slice step cannot be zero");
    }
    let adj = |v: i64, a: i64, b: i64| if v < 0 { (v + n).clamp(a, b) } else { v.clamp(a, b) };
    let (mut i, stop) = if step > 0 {
        (lo.map(|v| adj(v, 0, n)).unwrap_or(0), hi.map(|v| adj(v, 0, n)).unwrap_or(n))
    } else {
        (lo.map(|v| adj(v, -1, n - 1)).unwrap_or(n - 1), hi.map(|v| adj(v, -1, n - 1)).unwrap_or(-1))
    };
    let mut out = Vec::new();
    while (step > 0 && i < stop) || (step < 0 && i > stop) {
        out.push(i as usize);
        i += step;
    }
    Ok(out)
}

/// Result of a native method call.
pub struct Method {
    pub ret: Ty,
    /// New type for the receiver (joined into its place): `xs.append(1)`
    /// on a `list[Never]` widens it to `list[int]`.
    pub widen: Option<Ty>,
}

fn m(ret: Ty) -> Option<Result<Method, String>> {
    Some(Ok(Method { ret, widen: None }))
}

fn mw(ret: Ty, widen: Ty) -> Option<Result<Method, String>> {
    Some(Ok(Method { ret, widen: Some(widen) }))
}

fn err(msg: String) -> Option<Result<Method, String>> {
    Some(Err(msg))
}

/// `recv.name(args, **kwargs)` on a native receiver. `None`: not native
/// (the receiver is a Python object).
pub fn method(recv: Ty, name: &str, args: &[Ty], kwargs: &[(String, Ty)], boxing: bool) -> Option<Result<Method, String>> {
    let n = args.len();
    let kw = |k: &str| kwargs.iter().find(|(n, _)| n == k).map(|(_, t)| *t);
    let no_attr = || err(format!("'{}' object has no attribute '{name}'", recv.py_name().split('[').next().unwrap_or("")));
    let arity = |lo: usize, hi: usize| -> Result<(), String> {
        if (lo..=hi).contains(&n) {
            Ok(())
        } else {
            Err(format!("{}.{name}() takes {} argument(s) ({n} given)", recv.py_name().split('[').next().unwrap_or(""), if lo == hi { lo.to_string() } else { format!("{lo} to {hi}") }))
        }
    };
    macro_rules! check {
        ($e:expr) => {
            if let Err(e) = $e {
                return err(e);
            }
        };
    }
    if !kwargs.is_empty() && !(matches!(name, "sort" | "split" | "rsplit" | "splitlines" | "get" | "pop" | "replace" | "format")) {
        return err(format!("{name}() takes no keyword arguments natively"));
    }
    let elem_join = |e: Ty, t: Ty| ty_join(e, t, boxing).or(boxing.then_some(Ty::Obj));
    if recv == Ty::Str {
        let str_args = |k: usize| args.iter().take(k).all(|t| base(*t) == Ty::Str || *t == Ty::Obj);
        return match name {
            "upper" | "lower" | "casefold" | "swapcase" | "title" | "capitalize" => {
                check!(arity(0, 0));
                m(Ty::Str)
            }
            "strip" | "lstrip" | "rstrip" => {
                check!(arity(0, 1));
                m(Ty::Str)
            }
            "removeprefix" | "removesuffix" => {
                check!(arity(1, 1));
                m(Ty::Str)
            }
            "split" | "rsplit" => {
                check!(arity(0, 2));
                m(Ty::list(Ty::Str))
            }
            "splitlines" => {
                check!(arity(0, 1));
                m(Ty::list(Ty::Str))
            }
            "join" => {
                check!(arity(1, 1));
                match iter_elem(args[0]) {
                    Some(Ty::Str) | Some(Ty::Obj) => m(Ty::Str),
                    Some(t) if t.is_never() => m(Ty::Str),
                    Some(t) => err(format!("sequence item 0: expected str instance, {t} found")),
                    None => err(format!("can only join an iterable, not {}", args[0])),
                }
            }
            "replace" => {
                check!(arity(2, 3));
                m(Ty::Str)
            }
            "startswith" | "endswith" => {
                check!(arity(1, 1));
                match args[0] {
                    Ty::Str => m(Ty::Bool),
                    t if t.tuple_items().is_some_and(|ts| ts.iter().all(|x| *x == Ty::Str)) => m(Ty::Bool),
                    t if t.as_comp() == Some(Comp::TupleVar(Ty::Str)) => m(Ty::Bool),
                    t => err(format!("{name} first arg must be str or a tuple of str, not {t}")),
                }
            }
            "find" | "rfind" | "index" | "rindex" | "count" => {
                check!(arity(1, 1));
                if !str_args(1) {
                    return err(format!("must be str, not {}", args[0]));
                }
                m(Ty::Int)
            }
            "isdigit" | "isnumeric" | "isdecimal" | "isalpha" | "isalnum" | "isspace" | "isupper" | "islower" | "isidentifier" | "istitle" => {
                check!(arity(0, 0));
                m(Ty::Bool)
            }
            "center" | "ljust" | "rjust" => {
                check!(arity(1, 2));
                m(Ty::Str)
            }
            "zfill" => {
                check!(arity(1, 1));
                m(Ty::Str)
            }
            "expandtabs" => {
                check!(arity(0, 1));
                m(Ty::Str)
            }
            "partition" | "rpartition" => {
                check!(arity(1, 1));
                m(Ty::tuple(vec![Ty::Str, Ty::Str, Ty::Str]))
            }
            "format" => m(Ty::Str),
            // These keep CPython's implementation.
            "format_map" | "encode" | "translate" | "maketrans" => None,
            _ => no_attr(),
        };
    }
    let comp = recv.as_comp()?;
    match comp {
        Comp::List(e) => match name {
            "append" => {
                check!(arity(1, 1));
                match elem_join(e, args[0]) {
                    Some(j) => mw(Ty::None, Ty::list(j)),
                    None => err(format!("cannot append {} to a list[{e}] in native-only mode", args[0])),
                }
            }
            "insert" => {
                check!(arity(2, 2));
                if !int_like(base(args[0])) {
                    return err(format!("'{}' object cannot be interpreted as an integer", args[0]));
                }
                match elem_join(e, args[1]) {
                    Some(j) => mw(Ty::None, Ty::list(j)),
                    None => err(format!("cannot insert {} into a list[{e}]", args[1])),
                }
            }
            "extend" => {
                check!(arity(1, 1));
                match iter_elem(args[0]).and_then(|t| elem_join(e, t)) {
                    Some(j) => mw(Ty::None, Ty::list(j)),
                    None => err(format!("cannot extend a list[{e}] with {}", args[0])),
                }
            }
            "pop" => {
                check!(arity(0, 1));
                m(e)
            }
            "remove" | "index" | "count" => {
                check!(arity(1, 1));
                if elem_join(e, args[0]).is_none() {
                    return err(format!("{name}() of a {} with a {} value", recv, args[0]));
                }
                m(if name == "remove" { Ty::None } else { Ty::Int })
            }
            "sort" => {
                check!(arity(0, 0));
                if kw("key").is_some() {
                    return err("sort(key=...) needs a function value (closures are phase 4)".into());
                }
                m(Ty::None)
            }
            "reverse" | "clear" => {
                check!(arity(0, 0));
                m(Ty::None)
            }
            "copy" => {
                check!(arity(0, 0));
                m(recv)
            }
            _ => no_attr(),
        },
        Comp::Dict(k, v) => {
            let key_ok = |t: Ty| ty_join(k, t, false).is_some_and(|j| j == k || k.is_never()) || t == Ty::Obj;
            match name {
                "get" => {
                    check!(arity(1, 2));
                    if !key_ok(args[0]) {
                        return m(if n == 2 { args[1] } else { Ty::None });
                    }
                    if n == 1 {
                        m(Ty::opt(v))
                    } else {
                        match ty_join(v, args[1], boxing) {
                            Some(j) => m(j),
                            None => err(format!("dict.get default of type {} for values of type {v}", args[1])),
                        }
                    }
                }
                "pop" => {
                    check!(arity(1, 2));
                    if n == 1 {
                        m(v)
                    } else {
                        match ty_join(v, args[1], boxing) {
                            Some(j) => m(j),
                            None => err(format!("dict.pop default of type {} for values of type {v}", args[1])),
                        }
                    }
                }
                "setdefault" => {
                    check!(arity(2, 2));
                    let (Some(kk), Some(vv)) = (ty_join(k, args[0], false), elem_join(v, args[1])) else {
                        return err(format!("setdefault({}, {}) on a {recv}", args[0], args[1]));
                    };
                    mw(vv, Ty::dict(kk, vv))
                }
                "keys" => {
                    check!(arity(0, 0));
                    m(Ty::comp(Comp::View(pyfast_ir::ViewKind::Keys, k, v)))
                }
                "values" => {
                    check!(arity(0, 0));
                    m(Ty::comp(Comp::View(pyfast_ir::ViewKind::Values, k, v)))
                }
                "items" => {
                    check!(arity(0, 0));
                    m(Ty::comp(Comp::View(pyfast_ir::ViewKind::Items, k, v)))
                }
                "popitem" => {
                    check!(arity(0, 0));
                    m(Ty::tuple(vec![k, v]))
                }
                "update" => {
                    check!(arity(1, 1));
                    match ty_join(recv, args[0], boxing) {
                        Some(j) => mw(Ty::None, j),
                        None => err(format!("update of a {recv} with {}", args[0])),
                    }
                }
                "clear" => {
                    check!(arity(0, 0));
                    m(Ty::None)
                }
                "copy" => {
                    check!(arity(0, 0));
                    m(recv)
                }
                _ => no_attr(),
            }
        }
        Comp::Set(e) => match name {
            "add" => {
                check!(arity(1, 1));
                match ty_join(e, args[0], boxing).filter(|t| t.is_hashable()) {
                    Some(j) => mw(Ty::None, Ty::set(j)),
                    None => err(format!("cannot add {} to a set[{e}] natively", args[0])),
                }
            }
            "update" => {
                check!(arity(1, 1));
                match iter_elem(args[0]).and_then(|t| ty_join(e, t, boxing)).filter(|t| t.is_hashable()) {
                    Some(j) => mw(Ty::None, Ty::set(j)),
                    None => err(format!("cannot update a set[{e}] with {}", args[0])),
                }
            }
            "discard" | "remove" => {
                check!(arity(1, 1));
                m(Ty::None)
            }
            "pop" => {
                check!(arity(0, 0));
                m(e)
            }
            "clear" => {
                check!(arity(0, 0));
                m(Ty::None)
            }
            "copy" => {
                check!(arity(0, 0));
                m(recv)
            }
            "union" | "intersection" | "difference" | "symmetric_difference" => {
                check!(arity(1, 1));
                match ty_join(recv, args[0], boxing) {
                    Some(j) if j.set_elem().is_some() => m(j),
                    _ => err(format!("set.{name}() with {} natively", args[0])),
                }
            }
            "issubset" | "issuperset" | "isdisjoint" => {
                check!(arity(1, 1));
                if args[0].set_elem().is_none() {
                    return err(format!("set.{name}() with {} natively", args[0]));
                }
                m(Ty::Bool)
            }
            _ => no_attr(),
        },
        Comp::Tuple(_) | Comp::TupleVar(_) => match name {
            "index" | "count" => {
                check!(arity(1, 1));
                if recv.tuple_items().is_some() && recv.iter_elem().is_none() {
                    return err(format!("tuple.{name}() on a heterogeneous {recv}"));
                }
                m(Ty::Int)
            }
            _ => no_attr(),
        },
        _ => None,
    }
}

/// A CPython builtin called with native arguments. `None`: not native.
pub fn builtin(name: &str, args: &[Ty], kwargs: &[(String, Ty)], boxing: bool) -> Option<Result<Ty, String>> {
    let n = args.len();
    if args.contains(&Ty::Obj) && name != "format" {
        return None;
    }
    let only_kw = |allowed: &[&str]| kwargs.iter().all(|(k, _)| allowed.contains(&k.as_str()));
    let elem = |t: Ty| iter_elem(t);
    let numeric_sum = |e: Ty, start: Option<Ty>| -> Option<Result<Ty, String>> {
        let e = if e.is_never() { Ty::Int } else { e };
        let s = start.unwrap_or(Ty::Int);
        match (e, s) {
            (Ty::Int | Ty::Bool, Ty::Int | Ty::Bool) => Some(Ok(Ty::Int)),
            (Ty::Float | Ty::Int | Ty::Bool, Ty::Float | Ty::Int | Ty::Bool) => Some(Ok(Ty::Float)),
            _ => None,
        }
    };
    match name {
        "sorted" if n == 1 && only_kw(&["reverse"]) => {
            if kwargs.iter().any(|(k, _)| k == "key") {
                return None;
            }
            Some(elem(args[0]).map(Ty::list).ok_or_else(|| format!("'{}' object is not iterable", args[0])))
        }
        "sum" if (1..=2).contains(&n) && kwargs.is_empty() => numeric_sum(elem(args[0])?, args.get(1).copied()),
        "min" | "max" if n == 1 && kwargs.is_empty() => {
            let e = elem(args[0])?;
            Some(if e.is_never() { Err(format!("{name}() iterable argument is empty")) } else { Ok(e) })
        }
        "any" | "all" if n == 1 && kwargs.is_empty() => elem(args[0]).map(|_| Ok(Ty::Bool)),
        "list" if kwargs.is_empty() => match n {
            0 => Some(Ok(Ty::list(Ty::never()))),
            1 => elem(args[0]).map(|e| Ok(Ty::list(e))),
            _ => None,
        },
        "tuple" if kwargs.is_empty() => match n {
            0 => Some(Ok(Ty::tuple(vec![]))),
            1 => elem(args[0]).map(|e| Ok(Ty::tuple_var(e))),
            _ => None,
        },
        "set" if kwargs.is_empty() => match n {
            0 => Some(Ok(Ty::set(Ty::never()))),
            1 => elem(args[0]).filter(|e| e.is_hashable()).map(|e| Ok(Ty::set(e))),
            _ => None,
        },
        "dict" => {
            if n == 0 {
                if kwargs.is_empty() {
                    return Some(Ok(Ty::dict(Ty::never(), Ty::never())));
                }
                let v = join_all(kwargs.iter().map(|(_, t)| *t), boxing)?;
                return Some(Ok(Ty::dict(Ty::Str, v)));
            }
            if n == 1 && kwargs.is_empty() {
                if args[0].dict_kv().is_some() {
                    return Some(Ok(args[0]));
                }
                let e = elem(args[0])?;
                let items = e.tuple_items()?;
                if items.len() == 2 && items[0].is_hashable() {
                    return Some(Ok(Ty::dict(items[0], items[1])));
                }
            }
            None
        }
        "reversed" if n == 1 && kwargs.is_empty() => {
            if args[0].set_elem().is_some() || args[0].dict_kv().is_some() {
                return None;
            }
            elem(args[0]).map(|e| Ok(Ty::list(e)))
        }
        "enumerate" if (1..=2).contains(&n) && only_kw(&["start"]) => elem(args[0]).map(|e| Ok(Ty::list(Ty::tuple(vec![Ty::Int, e])))),
        "zip" if n >= 1 && kwargs.is_empty() => {
            let es: Option<Vec<Ty>> = args.iter().map(|a| elem(*a)).collect();
            es.map(|es| Ok(Ty::list(Ty::tuple(es))))
        }
        "range" if (1..=3).contains(&n) && kwargs.is_empty() => {
            Some(if args.iter().all(|a| int_like(base(*a))) { Ok(Ty::list(Ty::Int)) } else { Err("'float' object cannot be interpreted as an integer".into()) })
        }
        "repr" | "ascii" if n == 1 && kwargs.is_empty() => Some(Ok(Ty::Str)),
        "format" if (1..=2).contains(&n) && kwargs.is_empty() => {
            if n == 2 && base(args[1]) != Ty::Str {
                return Some(Err(format!("format() argument 2 must be str, not {}", args[1])));
            }
            Some(Ok(Ty::Str))
        }
        "ord" if n == 1 && kwargs.is_empty() => Some(if args[0] == Ty::Str { Ok(Ty::Int) } else { Err(format!("ord() expected string of length 1, but {} found", args[0])) }),
        "chr" if n == 1 && kwargs.is_empty() => Some(if int_like(base(args[0])) { Ok(Ty::Str) } else { Err(format!("'{}' object cannot be interpreted as an integer", args[0])) }),
        "divmod" if n == 2 && kwargs.is_empty() => {
            let (a, b) = (base(args[0]), base(args[1]));
            match (a, b) {
                _ if int_like(a) && int_like(b) => Some(Ok(Ty::tuple(vec![Ty::Int, Ty::Int]))),
                _ if a.is_numeric() && b.is_numeric() => Some(Ok(Ty::tuple(vec![Ty::Float, Ty::Float]))),
                _ => None,
            }
        }
        "round" if (1..=2).contains(&n) && kwargs.is_empty() => {
            let a = base(args[0]);
            match (a, n) {
                (Ty::Float, 1) => Some(Ok(Ty::Int)),
                (Ty::Float, 2) if args[1] == Ty::None => Some(Ok(Ty::Int)),
                (Ty::Float, 2) => Some(Ok(Ty::Float)),
                (Ty::Int | Ty::Bool, _) => Some(Ok(Ty::Int)),
                _ => None,
            }
        }
        "pow" if n == 2 && kwargs.is_empty() => None, // the operator handles `**`
        "hex" | "oct" | "bin" if n == 1 && kwargs.is_empty() && int_like(base(args[0])) => Some(Ok(Ty::Str)),
        "isinstance" if n == 2 => None, // decided by `isinstance_static`
        _ => None,
    }
}

/// `isinstance(v, cls)` for a native value and a builtin class name:
/// decided at compile time.
pub fn isinstance_static(v: Ty, cls: &str) -> Option<bool> {
    if v == Ty::Obj || v.as_opt().is_some() {
        return None;
    }
    let kind = match v {
        Ty::Int => "int",
        Ty::Float => "float",
        Ty::Str => "str",
        Ty::Bool => "bool",
        Ty::None => "NoneType",
        Ty::Obj => return None,
        Ty::C(_) => match v.as_comp()? {
            Comp::List(_) => "list",
            Comp::Dict(..) => "dict",
            Comp::Set(_) => "set",
            Comp::Tuple(_) | Comp::TupleVar(_) => "tuple",
            _ => return None,
        },
    };
    Some(match cls {
        "object" => true,
        "int" => kind == "int" || kind == "bool",
        "float" | "str" | "bool" | "list" | "dict" | "set" | "tuple" => kind == cls,
        _ => return None,
    })
}

/// Binary operators on native collections: `list + list`, `list * int`,
/// `tuple + tuple`, set algebra, `dict | dict`.
pub fn coll_binop(op: pyfast_ir::BinOp, l: Ty, r: Ty, boxing: bool) -> Option<Ty> {
    use pyfast_ir::BinOp::*;
    let (lc, rc) = (l.as_comp(), r.as_comp());
    match (op, &lc, &rc) {
        (Add, Some(Comp::List(_)), Some(Comp::List(_))) => ty_join(l, r, boxing),
        (Mul, Some(Comp::List(_)), None) if int_like(r) => Some(l),
        (Mul, None, Some(Comp::List(_))) if int_like(l) => Some(r),
        (Add, Some(Comp::Tuple(a)), Some(Comp::Tuple(b))) => Some(Ty::tuple(a.iter().chain(b).copied().collect())),
        (Add, Some(Comp::Tuple(_) | Comp::TupleVar(_)), Some(Comp::Tuple(_) | Comp::TupleVar(_))) => {
            let (a, b) = (iter_elem(l)?, iter_elem(r)?);
            Some(Ty::tuple_var(ty_join(a, b, boxing)?))
        }
        (Mul, Some(Comp::Tuple(_) | Comp::TupleVar(_)), None) if int_like(r) => Some(Ty::tuple_var(iter_elem(l)?)),
        (BitOr | BitAnd | Sub | BitXor, Some(Comp::Set(_)), Some(Comp::Set(_))) => ty_join(l, r, boxing),
        (BitOr, Some(Comp::Dict(..)), Some(Comp::Dict(..))) => ty_join(l, r, boxing),
        _ => None,
    }
}

/// `item in container` for native types.
pub fn contains_ok(item: Ty, container: Ty) -> bool {
    match container {
        Ty::Str => base(item) == Ty::Str,
        c => match c.as_comp() {
            Some(Comp::Dict(k, _)) => ty_join(k, item, false).is_some(),
            Some(Comp::List(e) | Comp::Set(e) | Comp::TupleVar(e)) => e.is_never() || ty_join(e, item, false).is_some() || e == Ty::Obj,
            Some(Comp::Tuple(_)) => iter_elem(c).is_some_and(|e| ty_join(e, item, false).is_some()),
            Some(Comp::View(..)) => iter_elem(c).is_some_and(|e| ty_join(e, item, false).is_some()),
            _ => false,
        },
    }
}

/// Non-retaining, non-mutating CPython callables that may receive a *copy*
/// of a native container without breaking Python semantics.
pub const READ_ONLY_CALLS: &[&str] = &[
    "json.dumps", "json.dump", "statistics.mean", "statistics.median", "statistics.mode", "statistics.stdev",
    "statistics.pstdev", "statistics.variance", "statistics.pvariance", "statistics.fmean", "statistics.geometric_mean",
    "statistics.harmonic_mean", "math.fsum", "math.prod", "math.dist", "math.hypot", "os.path.join", "pprint.pprint",
    "pprint.pformat", "repr", "str", "len", "print", "format", "hash", "id", "type", "isinstance", "frozenset", "bytes",
];
