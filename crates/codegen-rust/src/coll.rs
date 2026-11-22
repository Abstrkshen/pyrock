//! Code generation for native collections, strings and builtins.

use crate::{borrow, boxed, FnEmitter};
use pyfast_infer::native;
use pyfast_ir::*;

/// Rust code converting a value of type `from` to type `to`. Mutable
/// containers are copied here; inference widens places so that this only
/// happens for fresh values or at documented boundaries.
pub(crate) fn convert(code: &str, from: Ty, to: Ty) -> String {
    if from == to {
        return code.to_string();
    }
    if from.is_never() {
        return format!("{{ let __n: rt::Never = {code}; match __n {{}} }}");
    }
    if to == Ty::Obj {
        return boxed(code, from);
    }
    if let Some(x) = to.as_opt() {
        return match from {
            Ty::None if code == "()" => "None".into(),
            Ty::None => format!("{{ {code}; None }}"),
            f if f.as_opt().is_some() => {
                let fx = f.as_opt().unwrap();
                if fx == x { code.into() } else { format!("{code}.map(|__o| {})", convert("__o", fx, x)) }
            }
            f => format!("Some({})", convert(code, f, x)),
        };
    }
    if let Some(ms) = to.union_members() {
        let ur = to.rust();
        if from.class_id().is_some() || from.as_closure().is_some() {
            if let Some(target) = ms.iter().find(|m| **m == from) {
                return format!("{ur}::{}({code})", target.variant_name());
            }
            // The same function with wider captures: re-wrap is not possible (cells differ).
        }
        if let Some(fs) = from.union_members() {
            let fr = from.rust();
            let arms: Vec<String> = fs.iter().filter(|f| ms.contains(f)).map(|f| format!("{fr}::{0}(x) => {ur}::{0}(x)", f.variant_name())).collect();
            return format!("match {code} {{ {} }}", arms.join(", "));
        }
    }
    match (from.as_comp(), to.as_comp()) {
        // A record used as a plain dict: a copy.
        (Some(Comp::Record(_)), Some(Comp::Dict(..))) => {
            let own = native::record_dict(from, true).unwrap_or(to);
            convert(&format!("{code}.to_dict()?"), own, to)
        }
        // Same literal keys (the join rule): copy key by key; added keys keep
        // their insertion order.
        (Some(Comp::Record(fa)), Some(Comp::Record(fb))) => {
            let src = |k: &str| fa.iter().position(|f| f.key == k).unwrap();
            let req: Vec<String> = fb.iter().filter(|f| !f.optional).map(|f| convert(&format!("__r.f{}.clone()", src(f.key)), fa[src(f.key)].ty, f.ty)).collect();
            let mut code_out = format!("{{ let __s = {code}; let __r = __s.0.borrow(); let __n = {}::new({}); ", to.rust(), req.join(", "));
            let arms: Vec<String> = fa
                .iter()
                .enumerate()
                .filter(|(_, f)| f.optional)
                .map(|(i, f)| {
                    let j = fb.iter().position(|g| g.key == f.key).unwrap();
                    format!("{i} => __n.set_f{j}({}),", convert(&format!("__r.f{i}.clone().unwrap()"), f.ty, fb[j].ty))
                })
                .collect();
            if !arms.is_empty() {
                write!(code_out, "for &__i in &__r.order {{ match __i {{ {} _ => unreachable!() }} }} ", arms.join(" ")).unwrap();
            }
            code_out.push_str("__n }");
            code_out
        }
        // Generators and coroutines convert lazily (a wrapper converting each value).
        (Some(Comp::Gen(a)), Some(Comp::Gen(b))) => format!("{code}.map(|__e| -> PyResult<{}> {{ Ok({}) }})", b.rust(), convert("__e", a, b)),
        (Some(Comp::Coro(a)), Some(Comp::Coro(b))) => format!("{code}.map(|__e| -> PyResult<{}> {{ Ok({}) }})", b.rust(), convert("__e", a, b)),
        (Some(Comp::List(a)), Some(Comp::List(b))) => {
            format!("rt::List::from_vec({code}.to_vec().into_iter().map(|__e| {}).collect())", convert("__e", a, b))
        }
        (Some(Comp::Set(a)), Some(Comp::Set(b))) => format!("rt::Set::from_vec({code}.to_vec().into_iter().map(|__e| {}).collect())", convert("__e", a, b)),
        (Some(Comp::Dict(k1, v1)), Some(Comp::Dict(k2, v2))) => format!(
            "rt::Dict::from_pairs({code}.items_vec().into_iter().map(|(__k, __v)| ({}, {})).collect())",
            convert("__k", k1, k2),
            convert("__v", v1, v2)
        ),
        (Some(Comp::Tuple(xs)), Some(Comp::Tuple(ys))) if xs.len() == ys.len() => {
            let parts: Vec<String> = xs.iter().zip(&ys).enumerate().map(|(i, (x, y))| convert(&format!("__t.{i}"), *x, *y)).collect();
            format!("{{ let __t = {code}; ({},) }}", parts.join(", "))
        }
        (Some(Comp::Tuple(xs)), Some(Comp::TupleVar(y))) => {
            let parts: Vec<String> = xs.iter().enumerate().map(|(i, x)| convert(&format!("__t.{i}"), *x, y)).collect();
            if xs.is_empty() {
                "rt::Tup::from_vec(vec![])".into()
            } else {
                format!("{{ let __t = {code}; rt::Tup::from_vec(vec![{}]) }}", parts.join(", "))
            }
        }
        (Some(Comp::TupleVar(a)), Some(Comp::TupleVar(b))) => format!("rt::Tup::from_vec({code}.to_vec().into_iter().map(|__e| {}).collect())", convert("__e", a, b)),
        _ => crate::coerce_basic(code.to_string(), from, to),
    }
}

/// A value as a `rt::fmt::Arg` (for format specs, `%` and `str.format`).
pub(crate) fn to_arg(code: &str, t: Ty) -> String {
    match t {
        Ty::Int => format!("rt::fmt::Arg::Int({code})"),
        Ty::Float => format!("rt::fmt::Arg::Float({code})"),
        Ty::Bool => format!("rt::fmt::Arg::Bool({code})"),
        Ty::Str => format!("rt::fmt::Arg::Str({code})"),
        Ty::Obj => format!("rt::fmt::Arg::Py({code})"),
        Ty::None => format!("{{ {code}; rt::fmt::other(&(), \"NoneType\") }}"),
        t => match t.as_opt() {
            Some(inner) => format!("match {code} {{ Some(__x) => {}, None => rt::fmt::other(&(), \"NoneType\") }}", to_arg("__x", inner)),
            None => {
                let name = t.py_name().split('[').next().unwrap_or("object");
                format!("rt::fmt::other(&{}, {name:?})", borrow(code))
            }
        },
    }
}

/// `Vec<E>` of the elements of an iterable value of type `t`.
pub(crate) fn iter_vec(code: &str, t: Ty) -> String {
    match t {
        Ty::Str => format!("rt::coll::chars(&{})", borrow(code)),
        Ty::Obj => format!("rt::py::to_vec(&{})?", borrow(code)),
        _ => match t.as_comp() {
            Some(Comp::Dict(..)) => format!("{}.keys_vec()", code),
            Some(Comp::Gen(_)) => format!("{}.to_vec()?", code),
            Some(Comp::Tuple(ts)) => {
                if ts.is_empty() {
                    return "Vec::<rt::Never>::new()".into();
                }
                let el = native::iter_elem(t).unwrap_or(Ty::Obj);
                let parts: Vec<String> = ts.iter().enumerate().map(|(i, x)| convert(&format!("__t.{i}"), *x, el)).collect();
                format!("{{ let __t = {code}; vec![{}] }}", parts.join(", "))
            }
            _ => format!("{}.to_vec()", code),
        },
    }
}

/// `Vec<want>` of the elements of an iterable (converting each element).
pub(crate) fn iter_vec_as(code: &str, t: Ty, want: Ty) -> String {
    let el = native::iter_elem(t).unwrap_or(want);
    if el == want {
        return iter_vec(code, t);
    }
    if el.is_never() {
        return format!("{{ let _ = {code}; Vec::<{}>::new() }}", want.rust());
    }
    format!("{}.into_iter().map(|__e| {}).collect::<Vec<_>>()", iter_vec(code, t), convert("__e", el, want))
}

fn opt_i64(code: Option<String>) -> String {
    match code {
        Some(c) => format!("Some({c})"),
        None => "None".into(),
    }
}

impl FnEmitter<'_, '_> {
    /// Code for `e` as a value of type `to` (fresh literals are built
    /// directly at that type, so they never need a copy).
    pub(crate) fn expr_as(&mut self, e: &Expr, to: Ty) -> String {
        let et = self.ty(e);
        if et == to {
            return self.expr(e).0;
        }
        let literal = match (&e.kind, to.as_comp()) {
            (ExprKind::List(_), Some(Comp::List(_))) | (ExprKind::Set(_), Some(Comp::Set(_))) | (ExprKind::Dict(_), Some(Comp::Dict(..))) => true,
            (ExprKind::Dict(pairs), Some(Comp::Record(fs))) => {
                let req: Vec<&Field> = fs.iter().filter(|f| !f.optional).collect();
                pairs.len() == req.len() && pairs.iter().zip(&req).all(|((k, _), f)| matches!(&k.kind, ExprKind::Str(s) if s.as_str() == f.key))
            }
            (ExprKind::Tuple(items), Some(Comp::Tuple(ts))) => items.len() == ts.len(),
            (ExprKind::Tuple(_), Some(Comp::TupleVar(_))) => true,
            _ => false,
        };
        if literal {
            return self.build(e, to);
        }
        let (c, t) = self.expr(e);
        convert(&c, t, to)
    }

    /// Literal containers (their own type).
    pub(crate) fn literal(&mut self, e: &Expr, t: Ty) -> String {
        if t == Ty::Obj {
            return match &e.kind {
                ExprKind::List(items) => {
                    let a: Vec<String> = items.iter().map(|x| self.obj(x)).collect();
                    format!("rt::py::list(vec![{}])?", a.join(", "))
                }
                ExprKind::Tuple(items) => {
                    let a: Vec<String> = items.iter().map(|x| self.obj(x)).collect();
                    format!("rt::py::tuple(vec![{}])?", a.join(", "))
                }
                ExprKind::Set(items) => {
                    let a: Vec<String> = items.iter().map(|x| self.obj(x)).collect();
                    format!("rt::py::call(&rt::py::builtin(\"set\")?, vec![rt::py::list(vec![{}])?], vec![])?", a.join(", "))
                }
                ExprKind::Dict(pairs) => {
                    let a: Vec<String> = pairs.iter().map(|(k, v)| format!("({}, {})", self.obj(k), self.obj(v))).collect();
                    format!("rt::py::dict(vec![{}])?", a.join(", "))
                }
                _ => unreachable!(),
            };
        }
        self.build(e, t)
    }

    /// Construct a container literal at type `to` (which may differ from the
    /// literal's own type, e.g. `[1, 2]` stored into a `list[object]`).
    fn build(&mut self, e: &Expr, to: Ty) -> String {
        match (&e.kind, to.as_comp()) {
            (ExprKind::Dict(pairs), Some(Comp::Record(fs))) => {
                let req: Vec<Field> = fs.iter().filter(|f| !f.optional).copied().collect();
                let parts: Vec<String> = pairs.iter().zip(&req).map(|((_, v), f)| self.expr_as(v, f.ty)).collect();
                format!("{}::new({})", to.rust(), parts.join(", "))
            }
            (ExprKind::List(items), Some(Comp::List(b))) => {
                let parts: Vec<String> = items.iter().map(|x| self.expr_as(x, b)).collect();
                format!("rt::List::<{}>::from_vec(vec![{}])", b.rust(), parts.join(", "))
            }
            (ExprKind::Set(items), Some(Comp::Set(b))) => {
                let parts: Vec<String> = items.iter().map(|x| self.expr_as(x, b)).collect();
                format!("rt::Set::<{}>::from_vec(vec![{}])", b.rust(), parts.join(", "))
            }
            (ExprKind::Dict(pairs), Some(Comp::Dict(k, v))) => {
                let parts: Vec<String> = pairs.iter().map(|(a, b)| format!("({}, {})", self.expr_as(a, k), self.expr_as(b, v))).collect();
                format!("rt::Dict::<{}, {}>::from_pairs(vec![{}])", k.rust(), v.rust(), parts.join(", "))
            }
            (ExprKind::Tuple(items), Some(Comp::Tuple(ts))) if items.len() == ts.len() => {
                let parts: Vec<String> = items.iter().zip(&ts).map(|(x, t)| self.expr_as(x, *t)).collect();
                if parts.is_empty() { "rt::Tuple0".into() } else { format!("({},)", parts.join(", ")) }
            }
            (ExprKind::Tuple(items), Some(Comp::TupleVar(b))) => {
                let parts: Vec<String> = items.iter().map(|x| self.expr_as(x, b)).collect();
                format!("rt::Tup::<{}>::from_vec(vec![{}])", b.rust(), parts.join(", "))
            }
            _ => unreachable!("build {to} from a literal"),
        }
    }

    fn slice_args(&mut self, lo: &Option<Box<Expr>>, hi: &Option<Box<Expr>>, step: &Option<Box<Expr>>) -> String {
        let part = |me: &mut Self, x: &Option<Box<Expr>>| -> String {
            match x {
                None => "None".into(),
                Some(x) => {
                    let (c, t) = me.expr(x);
                    match t {
                        Ty::None => format!("{{ {c}; None }}"),
                        Ty::Obj => format!("rt::py::opt_index(&{})?", borrow(&c)),
                        t if t.as_opt().is_some() => c,
                        t => opt_i64(Some(crate::as_int(&c, t))),
                    }
                }
            }
        };
        let a = part(self, lo);
        let b = part(self, hi);
        let c = part(self, step);
        format!("{a}, {b}, {c}")
    }

    /// `obj[index]` on a native receiver.
    pub(crate) fn subscript(&mut self, obj: &Expr, index: &Expr, t: Ty) -> String {
        let (o, ot) = self.expr(obj);
        let ot = ot.as_opt().map(|_| ot).unwrap_or(ot);
        let o = if ot.as_opt().is_some() { format!("rt::unopt({o}, \"'NoneType' object is not subscriptable\")?") } else { o };
        let ot = ot.as_opt().unwrap_or(ot);
        // Tuples are read by value (fields move out); other receivers are borrowed.
        let o = if ot.tuple_items().is_some() { o } else { borrow(&o).to_string() };
        if let Some(fs) = ot.as_record() {
            if let ExprKind::Str(k) = &index.kind {
                return match fs.iter().position(|f| f.key == k.as_str()) {
                    Some(i) if fs[i].optional => format!("{o}.get_f{i}()?"),
                    Some(i) => format!("{o}.0.borrow().f{i}.clone()"),
                    None => format!("{{ let _ = {o}; rt::coll::missing_key({:?})? }}", crate::record::py_str_repr(k)),
                };
            }
            // A computed key on a record that is not a place: a copy as a dict.
            let d = native::record_dict(ot, true).unwrap_or(ot);
            let (i, it) = self.expr(index);
            let (k, _) = d.dict_kv().unwrap();
            return format!("{}.get(&{})?", convert(&o, ot, d), borrow(&convert(&i, it, k)));
        }
        if let ExprKind::Slice { lo, hi, step } = &index.kind {
            let args = self.slice_args(lo, hi, step);
            return match ot.as_comp() {
                _ if ot == Ty::Str => format!("rt::string::slice(&{}, {args})?", borrow(&o)),
                Some(Comp::List(_) | Comp::TupleVar(_)) => format!("{}.slice({args})?", o),
                Some(Comp::Tuple(ts)) => {
                    let lit = |x: &Option<Box<Expr>>| match x.as_deref().map(|e| &e.kind) {
                        None => Some(None),
                        Some(ExprKind::Int(v)) => Some(Some(*v)),
                        _ => None,
                    };
                    match (lit(lo), lit(hi), lit(step), t.tuple_items()) {
                        (Some(_), Some(_), Some(_), Some(out)) => {
                            // Positions are known at compile time: pick fields.
                            let pos = tuple_slice_positions(lo, hi, step, ts.len());
                            let parts: Vec<String> = pos.iter().map(|p| format!("__t.{p}")).collect();
                            if out.is_empty() {
                                format!("{{ let _ = {o}; rt::Tuple0 }}")
                            } else {
                                format!("{{ let __t = {o}; ({},) }}", parts.join(", "))
                            }
                        }
                        _ => format!("rt::Tup::from_vec({}).slice({args})?", iter_vec(&o, ot)),
                    }
                }
                _ => unreachable!("slice of {ot}"),
            };
        }
        let (i, it) = self.expr(index);
        match ot.as_comp() {
            _ if ot == Ty::Str => {
                let (i, it) = crate::unopt(i, it, "string indices must be integers, not 'NoneType'");
                format!("rt::string::index(&{}, {})?", borrow(&o), crate::as_int(&i, it))
            }
            Some(Comp::List(_) | Comp::TupleVar(_)) => {
                let (i, it) = crate::unopt(i, it, "list indices must be integers or slices, not NoneType");
                format!("{}.get({})?", o, crate::as_int(&i, it))
            }
            Some(Comp::Tuple(ts)) => match index.kind {
                ExprKind::Int(v) => {
                    let n = ts.len() as i64;
                    let j = if v < 0 { v + n } else { v };
                    format!("({o}).{j}.clone()")
                }
                _ => {
                    let idx = crate::as_int(&i, it);
                    let arms: Vec<String> = (0..ts.len()).map(|k| format!("{k} => __t.{k}")).collect();
                    format!(
                        "{{ let __t = {o}; let __n = {}_i64; let __i = {idx}; let __j = if __i < 0 {{ __i + __n }} else {{ __i }}; match __j {{ {}, _ => return Err(rt::Exc::new(\"IndexError\", \"tuple index out of range\")) }} }}",
                        ts.len(),
                        arms.join(", ")
                    )
                }
            },
            Some(Comp::Dict(k, _)) => format!("{}.get(&{})?", o, borrow(&convert(&i, it, k))),
            _ => unreachable!("subscript of {ot}"),
        }
    }

    /// `obj[index] = value` on a native receiver.
    pub(crate) fn set_item(&mut self, obj: &Expr, index: &Expr, value: &Expr, ind: &str) -> String {
        let ot = self.ty(obj);
        match ot.as_comp() {
            Some(Comp::List(e)) => {
                if let ExprKind::Slice { lo, hi, step } = &index.kind {
                    let (v, vt) = self.expr(value);
                    let items = iter_vec(&v, vt);
                    let ve = native::iter_elem(vt).unwrap_or(e);
                    let items = if ve == e { items } else { format!("{items}.into_iter().map(|__e| {}).collect()", convert("__e", ve, e)) };
                    let o = borrow(&self.expr(obj).0).to_string();
                    let args = self.slice_args(lo, hi, step);
                    return format!("{ind}{{\n{ind}    let __v = {items};\n{ind}    {o}.set_slice({args}, __v)?;\n{ind}}}");
                }
                // Python evaluates the value first, then the target.
                let v = self.expr_as(value, e);
                let o = borrow(&self.expr(obj).0).to_string();
                let (i, it) = self.expr(index);
                format!("{ind}{{\n{ind}    let __v = {v};\n{ind}    {o}.set({}, __v)?;\n{ind}}}", crate::as_int(&i, it))
            }
            Some(Comp::Dict(k, vty)) => {
                let v = self.expr_as(value, vty);
                let o = borrow(&self.expr(obj).0).to_string();
                let kc = self.expr_as(index, k);
                format!("{ind}{{\n{ind}    let __v = {v};\n{ind}    {o}.set({kc}, __v);\n{ind}}}")
            }
            Some(Comp::Record(fs)) => {
                let ExprKind::Str(k) = &index.kind else { unreachable!("computed key on a record place") };
                let i = fs.iter().position(|f| f.key == k.as_str()).expect("record key");
                let v = self.expr_as(value, fs[i].ty);
                let o = borrow(&self.expr(obj).0).to_string();
                if fs[i].optional {
                    format!("{ind}{{\n{ind}    let __v = {v};\n{ind}    {o}.set_f{i}(__v);\n{ind}}}")
                } else {
                    format!("{ind}{{\n{ind}    let __v = {v};\n{ind}    {o}.0.borrow_mut().f{i} = __v;\n{ind}}}")
                }
            }
            _ => unreachable!("set_item on {ot}"),
        }
    }

    /// `del obj[index]`
    pub(crate) fn del_item(&mut self, obj: &Expr, index: &Expr) -> String {
        let (o, ot) = self.expr(obj);
        let o = if ot == Ty::Obj { o } else { borrow(&o).to_string() };
        if ot == Ty::Obj {
            if let ExprKind::Slice { .. } = index.kind {
                let i = self.obj(index);
                return format!("rt::py::delitem(&{}, {i})?;", borrow(&o));
            }
            let i = self.obj(index);
            return format!("rt::py::delitem(&{}, {i})?;", borrow(&o));
        }
        if let (Some(fs), ExprKind::Str(k)) = (ot.as_record(), &index.kind) {
            let i = fs.iter().position(|f| f.key == k.as_str()).expect("optional record key");
            return format!("{o}.del_f{i}()?;");
        }
        match ot.as_comp() {
            Some(Comp::List(_)) => {
                if let ExprKind::Slice { lo, hi, step } = &index.kind {
                    let args = self.slice_args(lo, hi, step);
                    return format!("{o}.del_slice({args})?;");
                }
                let (i, it) = self.expr(index);
                format!("{o}.del({})?;", crate::as_int(&i, it))
            }
            Some(Comp::Dict(k, _)) => {
                let kc = self.expr_as(index, k);
                format!("{o}.del(&{kc})?;")
            }
            _ => unreachable!("del on {ot}"),
        }
    }

    /// `for var in iter:` over a native iterable: returns (loop head, element expression).
    pub(crate) fn for_native(&mut self, iter: &Expr, it: Ty, ind: &str, out: &mut String) -> (String, String) {
        let (src, _) = self.expr(iter);
        let n = self.fresh();
        match it.as_comp() {
            Some(Comp::List(_) | Comp::TupleVar(_)) => {
                writeln!(out, "{ind}let __c{n} = {src};\n{ind}let mut __i{n} = 0usize;").unwrap();
                (format!("while let Some(__v) = {{ let __x = __c{n}.at(__i{n}); __i{n} += 1; __x }}"), "__v".into())
            }
            Some(Comp::Dict(..)) => {
                writeln!(out, "{ind}let mut __c{n} = {src}.cursor();").unwrap();
                (format!("while let Some((__v, _)) = __c{n}.next()?"), "__v".into())
            }
            Some(Comp::Set(_) | Comp::View(..)) => {
                writeln!(out, "{ind}let mut __c{n} = {src}.cursor();").unwrap();
                (format!("while let Some(__v) = __c{n}.next()?"), "__v".into())
            }
            // Lazily: a generator may be infinite.
            Some(Comp::Gen(_)) => {
                writeln!(out, "{ind}let __c{n} = {src};").unwrap();
                (format!("while let Some(__v) = __c{n}.next()?"), "__v".into())
            }
            _ => {
                writeln!(out, "{ind}let __c{n} = {};", iter_vec(&src, it)).unwrap();
                (format!("for __v in __c{n}"), "__v".into())
            }
        }
    }

    /// A native method call.
    pub(crate) fn method(&mut self, obj: &Expr, recv: Ty, name: &str, args: &[Expr], kwargs: &[(String, Expr)], t: Ty) -> String {
        let (o, ot) = self.expr(obj);
        // A record that is not a place, used like a dict: a copy.
        let (o, ot) = if ot.as_record().is_some() { (convert(&o, ot, recv), recv) } else { (o, ot) };
        let o = if ot.as_opt().is_some() { format!("rt::unopt({o}, \"'NoneType' object has no attribute '{name}'\")?") } else { o };
        let o = borrow(&o).to_string();
        let vals: Vec<(String, Ty)> = args.iter().map(|a| self.expr(a)).collect();
        let kw = |k: &str| kwargs.iter().position(|(n, _)| n == k);
        let kwv: Vec<(String, Ty)> = kwargs.iter().map(|(_, v)| self.expr(v)).collect();
        let b = |c: &str| borrow(c).to_string();
        let int = |v: &(String, Ty)| crate::as_int(&v.0, v.1);
        if recv == Ty::Str {
            let s = b(&o);
            let arg_str = |v: &(String, Ty)| format!("&{}", borrow(&v.0));
            let opt_str = |v: Option<&(String, Ty)>| match v {
                None => "None".to_string(),
                Some((c, Ty::None)) => format!("{{ {c}; None }}"),
                Some(v) => format!("Some({})", arg_str(v)),
            };
            return match name {
                "upper" | "lower" | "casefold" | "swapcase" | "title" | "capitalize" => format!("rt::string::{name}(&{s})"),
                "strip" | "lstrip" | "rstrip" => format!("rt::string::{name}(&{s}, {})", opt_str(vals.first())),
                "removeprefix" | "removesuffix" => format!("rt::string::{name}(&{s}, {})", arg_str(&vals[0])),
                "split" | "rsplit" => {
                    let sep = vals.first().cloned().or_else(|| kw("sep").map(|i| kwv[i].clone()));
                    let maxs = vals.get(1).cloned().or_else(|| kw("maxsplit").map(|i| kwv[i].clone()));
                    let m = maxs.map(|v| int(&v)).unwrap_or_else(|| "-1".into());
                    format!("rt::string::{name}(&{s}, {}, {m})?", opt_str(sep.as_ref()))
                }
                "splitlines" => {
                    let keep = vals.first().cloned().or_else(|| kw("keepends").map(|i| kwv[i].clone()));
                    format!("rt::string::splitlines(&{s}, {})", keep.map(|v| crate::truthy(&v.0, v.1)).unwrap_or_else(|| "false".into()))
                }
                "join" => format!("rt::string::join(&{s}, &{})", iter_vec_as(&vals[0].0, vals[0].1, Ty::Str)),
                "replace" => {
                    let count = vals.get(2).cloned().or_else(|| kw("count").map(|i| kwv[i].clone()));
                    format!("rt::string::replace(&{s}, {}, {}, {})", arg_str(&vals[0]), arg_str(&vals[1]), count.map(|v| int(&v)).unwrap_or_else(|| "-1".into()))
                }
                "startswith" | "endswith" => {
                    let (c, at) = &vals[0];
                    if *at == Ty::Str {
                        format!("rt::string::{name}(&{s}, &{})", borrow(c))
                    } else {
                        format!("{{ let __s = {s}.clone(); {}.into_iter().any(|__p: rt::Str| rt::string::{name}(&__s, &__p)) }}", iter_vec(c, *at))
                    }
                }
                "find" | "rfind" | "count" => format!("rt::string::{name}(&{s}, {})", arg_str(&vals[0])),
                "index" => format!("rt::string::index_of(&{s}, {})?", arg_str(&vals[0])),
                "rindex" => format!("rt::string::rindex_of(&{s}, {})?", arg_str(&vals[0])),
                "isdigit" | "isnumeric" | "isdecimal" | "isalpha" | "isalnum" | "isspace" | "isupper" | "islower" | "isidentifier" | "istitle" => {
                    format!("rt::string::{name}(&{s})")
                }
                "center" | "ljust" | "rjust" => format!("rt::string::{name}(&{s}, {}, {})?", int(&vals[0]), opt_str(vals.get(1))),
                "zfill" => format!("rt::string::zfill(&{s}, {})", int(&vals[0])),
                "expandtabs" => format!("rt::string::expandtabs(&{s}, {})", vals.first().map(int).unwrap_or_else(|| "8".into())),
                "partition" => format!("rt::string::partition(&{s}, {}, false)?", arg_str(&vals[0])),
                "format" => {
                    let a: Vec<String> = vals.iter().map(|(c, t)| to_arg(c, *t)).collect();
                    let k: Vec<String> = kwargs.iter().zip(&kwv).map(|((n, _), (c, t))| format!("({n:?}, {})", to_arg(c, *t))).collect();
                    format!("rt::fmt::format_method(&{s}, &[{}], &[{}])?", a.join(", "), k.join(", "))
                }
                "rpartition" => format!("rt::string::partition(&{s}, {}, true)?", arg_str(&vals[0])),
                _ => unreachable!("str.{name}"),
            };
        }
        let comp = recv.as_comp().unwrap();
        match comp {
            Comp::List(e) => match name {
                "append" => format!("{o}.append({})", self.expr_as(&args[0], e)),
                "insert" => {
                    let i = int(&vals[0]);
                    format!("{o}.insert({i}, {})", self.expr_as(&args[1], e))
                }
                "extend" => {
                    let (c, ct) = &vals[0];
                    let ve = native::iter_elem(*ct).unwrap_or(e);
                    let items = iter_vec(c, *ct);
                    let items = if ve == e { items } else { format!("{items}.into_iter().map(|__e| {}).collect()", convert("__e", ve, e)) };
                    format!("{{ let __v = {items}; {o}.extend(__v) }}")
                }
                "pop" => format!("{o}.pop({})?", opt_i64(vals.first().map(int))),
                "remove" => format!("{o}.remove(&{})?", convert(&vals[0].0, vals[0].1, e)),
                "index" => format!("{o}.index(&{})?", convert(&vals[0].0, vals[0].1, e)),
                "count" => format!("{o}.count(&{})", convert(&vals[0].0, vals[0].1, e)),
                "sort" => {
                    let rev = kw("reverse").map(|i| crate::truthy(&kwv[i].0, kwv[i].1)).unwrap_or_else(|| "false".into());
                    match kwargs.iter().find(|(n, _)| n == "key") {
                        Some((_, k)) => self.sort_by_key(&o, e, k, rev),
                        None => format!("{o}.sort({rev})?"),
                    }
                }
                "reverse" | "clear" => format!("{o}.{name}()"),
                "copy" => format!("{o}.copy()"),
                _ => unreachable!("list.{name}"),
            },
            Comp::Dict(k, v) => match name {
                "get" => {
                    let key = convert(&vals[0].0, vals[0].1, k);
                    let key = borrow(&key).to_string();
                    if vals.len() == 1 {
                        // (also for `object` values, where a missing key is Python's None)
                        format!("match {o}.get_opt(&{key}) {{ Some(__x) => {}, None => {} }}", convert("__x", v, t), convert("()", Ty::None, t))
                    } else {
                        let d = convert(&vals[1].0, vals[1].1, t);
                        format!("{{ let __d = {d}; match {o}.get_opt(&{key}) {{ Some(__x) => {}, None => __d }} }}", convert("__x", v, t))
                    }
                }
                "pop" => {
                    let key = convert(&vals[0].0, vals[0].1, k);
                    let key = borrow(&key).to_string();
                    if vals.len() == 1 {
                        format!("{o}.pop(&{key})?")
                    } else {
                        let d = convert(&vals[1].0, vals[1].1, t);
                        format!("{{ let __d = {d}; match {o}.get_opt(&{key}) {{ Some(_) => {}, None => __d }} }}", convert(&format!("{o}.pop(&{key})?"), v, t))
                    }
                }
                "setdefault" => format!("{o}.setdefault({}, {})", self.expr_as(&args[0], k), self.expr_as(&args[1], v)),
                "keys" => format!("rt::coll::Keys({o}.clone())"),
                "values" => format!("rt::coll::Values({o}.clone())"),
                "items" => format!("rt::coll::Items({o}.clone())"),
                "popitem" => format!("{o}.popitem()?"),
                "update" => format!("{o}.update(&{})", convert(&vals[0].0, vals[0].1, recv)),
                "clear" => format!("{o}.clear()"),
                "copy" => format!("{o}.copy()"),
                _ => unreachable!("dict.{name}"),
            },
            Comp::Set(e) => match name {
                "add" => format!("{o}.add({})", self.expr_as(&args[0], e)),
                "update" => format!("{o}.update({})", iter_vec(&vals[0].0, vals[0].1)),
                "discard" => format!("{o}.discard(&{})", convert(&vals[0].0, vals[0].1, e)),
                "remove" => format!("{o}.remove(&{})?", convert(&vals[0].0, vals[0].1, e)),
                "pop" => format!("{o}.pop()?"),
                "clear" => format!("{o}.clear()"),
                "copy" => format!("{o}.copy()"),
                "union" | "intersection" | "difference" | "symmetric_difference" | "issubset" | "issuperset" | "isdisjoint" => {
                    format!("{}.{name}(&{})", convert(&o, recv, if t == Ty::Bool { recv } else { t }), convert(&vals[0].0, vals[0].1, if t == Ty::Bool { recv } else { t }))
                }
                _ => unreachable!("set.{name}"),
            },
            Comp::Tuple(_) | Comp::TupleVar(_) => {
                let e = native::iter_elem(recv).unwrap();
                let tup = if matches!(comp, Comp::TupleVar(_)) { o.clone() } else { format!("rt::Tup::from_vec({})", iter_vec(&o, recv)) };
                match name {
                    "index" => format!("{tup}.index(&{})?", convert(&vals[0].0, vals[0].1, e)),
                    "count" => format!("{tup}.count(&{})", convert(&vals[0].0, vals[0].1, e)),
                    _ => unreachable!("tuple.{name}"),
                }
            }
            _ => unreachable!("method on {recv}"),
        }
    }

    /// A CPython builtin with a native implementation.
    pub(crate) fn native_builtin(&mut self, name: &str, args: &[Expr], kwargs: &[(String, Expr)], t: Ty) -> String {
        let vals: Vec<(String, Ty)> = args.iter().map(|a| self.expr(a)).collect();
        // Records (not places) passed to builtins that need a dict: a copy.
        let vals: Vec<(String, Ty)> = vals
            .into_iter()
            .map(|(c, vt)| match native::record_dict(vt, true) {
                Some(d) if !pyfast_infer::spec::RECORD_BUILTINS.contains(&name) => (convert(&c, vt, d), d),
                _ => (c, vt),
            })
            .collect();
        let kw = |k: &str| kwargs.iter().position(|(n, _)| n == k);
        let kwv: Vec<(String, Ty)> = kwargs.iter().map(|(_, v)| self.expr(v)).collect();
        let elems = |v: &(String, Ty)| iter_vec(&v.0, v.1);
        match name {
            "sorted" => {
                let rev = kw("reverse").map(|i| crate::truthy(&kwv[i].0, kwv[i].1)).unwrap_or_else(|| "false".into());
                format!("rt::coll::sorted({}, {rev})?", elems(&vals[0]))
            }
            "sum" => {
                let e = native::iter_elem(vals[0].1).unwrap_or(Ty::Int);
                let e = if e.is_never() { Ty::Int } else { e };
                let elems = |v: &(String, Ty)| iter_vec_as(&v.0, v.1, e);
                if t == Ty::Int {
                    let start = vals.get(1).map(|v| crate::as_int(&v.0, v.1)).unwrap_or_else(|| "0".into());
                    let items = if e == Ty::Bool { format!("{}.into_iter().map(|b| b as i64).collect()", elems(&vals[0])) } else { elems(&vals[0]) };
                    format!("rt::coll::sum_int({items}, {start})?")
                } else {
                    let start = vals.get(1).map(|v| crate::as_float(&v.0, v.1)).unwrap_or_else(|| "0.0".into());
                    let items = if e == Ty::Float { elems(&vals[0]) } else { format!("{}.into_iter().map(|x| {}).collect()", elems(&vals[0]), crate::as_float("x", e)) };
                    format!("rt::coll::sum_float({items}, {start})")
                }
            }
            "min" | "max" if kw("default").is_some() => {
                let d = &kwv[kw("default").unwrap()];
                let e = native::iter_elem(vals[0].1).unwrap_or(t);
                if e.is_never() {
                    format!("{{ let _ = {}; {} }}", vals[0].0, convert(&d.0, d.1, t))
                } else {
                    format!(
                        "{{ let __v = {}; if __v.is_empty() {{ {} }} else {{ let __m = rt::coll::min_of(__v, {})?; {} }} }}",
                        elems(&vals[0]),
                        convert(&d.0, d.1, t),
                        name == "max",
                        convert("__m", e, t)
                    )
                }
            }
            "min" | "max" => format!("rt::coll::min_of({}, {})?", elems(&vals[0]), name == "max"),
            "any" | "all" => format!("rt::coll::{name}(&{})", elems(&vals[0])),
            "list" => match vals.first() {
                None => format!("rt::List::<{}>::new()", t.list_elem().unwrap().rust()),
                Some(v) => format!("rt::List::from_vec({})", elems(v)),
            },
            "tuple" => match vals.first() {
                None => "rt::Tuple0".into(),
                Some(v) => format!("rt::Tup::from_vec({})", elems(v)),
            },
            "set" => match vals.first() {
                None => format!("rt::Set::<{}>::new()", t.set_elem().unwrap().rust()),
                Some(v) => format!("rt::Set::from_vec({})", elems(v)),
            },
            "dict" => {
                let (k, v) = t.dict_kv().unwrap();
                if vals.is_empty() {
                    let pairs: Vec<String> = kwargs.iter().zip(&kwv).map(|((n, _), (c, ct))| format!("(rt::Str::from({n:?}), {})", convert(c, *ct, v))).collect();
                    return format!("rt::Dict::<{}, {}>::from_pairs(vec![{}])", k.rust(), v.rust(), pairs.join(", "));
                }
                if vals[0].1.dict_kv().is_some() {
                    return format!("{}.copy()", vals[0].0);
                }
                format!("rt::Dict::from_pairs({})", elems(&vals[0]))
            }
            "reversed" => format!("{{ let mut __v = {}; __v.reverse(); rt::List::from_vec(__v) }}", elems(&vals[0])),
            "next" => {
                let e = native::iter_elem(vals[0].1).unwrap_or(t);
                match vals.get(1) {
                    Some(d) => format!("match {}.next()? {{ Some(__v) => {}, None => {} }}", vals[0].0, convert("__v", e, t), convert(&d.0, d.1, t)),
                    None => format!("match {}.next()? {{ Some(__v) => {}, None => return Err(rt::Exc::new(\"StopIteration\", \"\")) }}", vals[0].0, convert("__v", e, t)),
                }
            }
            "enumerate" if native::is_gen(t) => {
                let start = vals.get(1).or(kw("start").map(|i| &kwv[i])).map(|v| crate::as_int(&v.0, v.1)).unwrap_or_else(|| "0".into());
                format!("rt::gen::enumerate({}, {start})", vals[0].0)
            }
            "zip" if native::is_gen(t) => {
                let gens: Vec<String> = vals.iter().map(|v| if native::is_gen(v.1) { v.0.clone() } else { format!("rt::gen::Gen::from_vec({})", elems(v)) }).collect();
                format!("rt::gen::zip{}({})", gens.len(), gens.join(", "))
            }
            "enumerate" => {
                let start = vals.get(1).or(kw("start").map(|i| &kwv[i])).map(|v| crate::as_int(&v.0, v.1)).unwrap_or_else(|| "0".into());
                format!("{{ let __s: i64 = {start}; rt::List::from_vec({}.into_iter().enumerate().map(|(i, x)| (__s + i as i64, x)).collect()) }}", elems(&vals[0]))
            }
            "zip" => {
                let vs: Vec<String> = vals.iter().map(elems).collect();
                let lets: Vec<String> = vs.iter().enumerate().map(|(i, v)| format!("let __z{i} = {v};")).collect();
                let n = vals.len();
                let len = (0..n).map(|i| format!("__z{i}.len()")).collect::<Vec<_>>();
                let iters: Vec<String> = (0..n).map(|i| format!("let mut __it{i} = __z{i}.into_iter();")).collect();
                let items: Vec<String> = (0..n).map(|i| format!("__it{i}.next().unwrap()")).collect();
                format!(
                    "{{ {} let __n = [{}].into_iter().min().unwrap_or(0); {} rt::List::from_vec((0..__n).map(|_| ({},)).collect()) }}",
                    lets.join(" "),
                    len.join(", "),
                    iters.join(" "),
                    items.join(", ")
                )
            }
            "range" => {
                let ints: Vec<String> = vals.iter().map(|v| crate::as_int(&v.0, v.1)).collect();
                let (a, b, c) = match ints.len() {
                    1 => ("0".to_string(), ints[0].clone(), "1".to_string()),
                    2 => (ints[0].clone(), ints[1].clone(), "1".to_string()),
                    _ => (ints[0].clone(), ints[1].clone(), ints[2].clone()),
                };
                format!("rt::List::from_vec(rt::coll::range_vec({a}, {b}, {c})?)")
            }
            "repr" => format!("rt::conv::repr_of(&{})", borrow(&vals[0].0)),
            "format" => {
                let arg = to_arg(&vals[0].0, vals[0].1);
                match vals.get(1) {
                    Some((sp, _)) => format!("rt::fmt::format(&{arg}, &{})?", borrow(sp)),
                    None => format!("rt::fmt::format(&{arg}, \"\")?"),
                }
            }
            "ascii" => format!("rt::conv::ascii_of(&{})", borrow(&vals[0].0)),
            "ord" => format!("rt::string::ord(&{})?", borrow(&vals[0].0)),
            "chr" => format!("rt::string::chr({})?", crate::as_int(&vals[0].0, vals[0].1)),
            "divmod" => {
                let (a, at) = &vals[0];
                let (b, bt) = &vals[1];
                if t.tuple_items().is_some_and(|ts| ts[0] == Ty::Int) {
                    format!("{{ let (__a, __b) = ({}, {}); (rt::int::floordiv(__a, __b)?, rt::int::modulo(__a, __b)?) }}", crate::as_int(a, *at), crate::as_int(b, *bt))
                } else {
                    format!("{{ let (__a, __b) = ({}, {}); (rt::float::floordiv(__a, __b)?, rt::float::modulo(__a, __b)?) }}", crate::as_float(a, *at), crate::as_float(b, *bt))
                }
            }
            "round" => {
                let (a, at) = &vals[0];
                match (at, t) {
                    (Ty::Int | Ty::Bool, _) => crate::as_int(a, *at),
                    (_, Ty::Int) => format!("rt::conv::round_int({a})?"),
                    _ => format!("rt::conv::round_to({a}, {})", crate::as_int(&vals[1].0, vals[1].1)),
                }
            }
            "hex" | "oct" | "bin" => format!("rt::conv::{name}({})", crate::as_int(&vals[0].0, vals[0].1)),
            _ => unreachable!("native builtin {name}"),
        }
    }

    /// Binary operator on native collections.
    pub(crate) fn coll_binop(&mut self, op: BinOp, l: &str, lt: Ty, r: &str, rt: Ty, t: Ty) -> String {
        use BinOp::*;
        let (lc, rc) = (lt.as_comp(), rt.as_comp());
        match (op, lc, rc) {
            (Add, Some(Comp::List(_)), _) => format!("{}.concat(&{})", convert(l, lt, t), convert(r, rt, t)),
            (Mul, Some(Comp::List(_)), _) => format!("{l}.repeat({})", crate::as_int(r, rt)),
            (Mul, _, Some(Comp::List(_))) => format!("{r}.repeat({})", crate::as_int(l, lt)),
            (Add, Some(Comp::Tuple(a)), Some(Comp::Tuple(b))) if t.tuple_items().is_some() => {
                let mut parts: Vec<String> = (0..a.len()).map(|i| format!("__a.{i}")).collect();
                parts.extend((0..b.len()).map(|i| format!("__b.{i}")));
                if parts.is_empty() {
                    format!("{{ let _ = ({l}, {r}); rt::Tuple0 }}")
                } else {
                    format!("{{ let __a = {l}; let __b = {r}; ({},) }}", parts.join(", "))
                }
            }
            (Add, _, _) => format!("{}.concat(&{})", convert(l, lt, t), convert(r, rt, t)),
            (Mul, _, _) => {
                let e = t.as_comp();
                let _ = e;
                format!("{{ let __v = {}; let __n = {}; rt::Tup::from_vec((0..__n.max(0)).flat_map(|_| __v.iter().cloned()).collect::<Vec<_>>()) }}", iter_vec(l, lt), crate::as_int(r, rt))
            }
            (BitOr, Some(Comp::Set(_)), _) => format!("{}.union(&{})", convert(l, lt, t), convert(r, rt, t)),
            (BitAnd, Some(Comp::Set(_)), _) => format!("{}.intersection(&{})", convert(l, lt, t), convert(r, rt, t)),
            (Sub, Some(Comp::Set(_)), _) => format!("{}.difference(&{})", convert(l, lt, t), convert(r, rt, t)),
            (BitXor, Some(Comp::Set(_)), _) => format!("{}.symmetric_difference(&{})", convert(l, lt, t), convert(r, rt, t)),
            (BitOr, Some(Comp::Dict(..)), _) => format!("{}.merge(&{})", convert(l, lt, t), convert(r, rt, t)),
            _ => unreachable!("collection operator {} on {lt} and {rt}", op.symbol()),
        }
    }

    /// `a, *b, c = value`: a fixed tuple of the unpacked items.
    pub(crate) fn unpack(&mut self, value: &Expr, n: usize, star: Option<usize>, t: Ty) -> String {
        let (v, vt) = self.expr(value);
        let items = t.tuple_items().unwrap();
        if let Some(ts) = vt.tuple_items() {
            let Some(k) = star else { return v };
            let after = n - 1 - k;
            let me = items[k].list_elem().unwrap();
            let mut parts: Vec<String> = (0..k).map(|i| format!("__t.{i}")).collect();
            let mid: Vec<String> = (k..ts.len() - after).map(|i| convert(&format!("__t.{i}"), ts[i], me)).collect();
            parts.push(format!("rt::List::<{}>::from_vec(vec![{}])", me.rust(), mid.join(", ")));
            parts.extend((ts.len() - after..ts.len()).map(|i| format!("__t.{i}")));
            return format!("{{ let __t = {v}; ({},) }}", parts.join(", "));
        }
        let mut parts = Vec::new();
        for i in 0..n {
            if Some(i) == star {
                parts.push(format!("rt::List::from_vec(__it.by_ref().take(__n - {}).collect())", n - 1));
            } else {
                parts.push("__it.next().unwrap()".to_string());
            }
        }
        format!(
            "{{ let __v = {}; let __n = __v.len(); rt::coll::unpack_check(__n, {n}, {})?; let mut __it = __v.into_iter(); ({},) }}",
            iter_vec(&v, vt),
            star.is_some(),
            parts.join(", ")
        )
    }

    /// A comprehension, built eagerly in a block expression.
    pub(crate) fn comp(&mut self, kind: CompKind, elt: &Expr, val: Option<&Expr>, gens: &[CompFor], t: Ty) -> String {
        let r = format!("__r{}", self.fresh());
        let mut code = String::new();
        let empty = match kind {
            CompKind::List | CompKind::Gen => format!("rt::List::<{}>::new()", t.list_elem().unwrap().rust()),
            CompKind::Set => format!("rt::Set::<{}>::new()", t.set_elem().unwrap().rust()),
            CompKind::Dict => {
                let (k, v) = t.dict_kv().unwrap();
                format!("rt::Dict::<{}, {}>::new()", k.rust(), v.rust())
            }
        };
        write!(code, "{{ let {r} = {empty}; ").unwrap();
        let mut closers = String::new();
        for g in gens {
            let it = self.ty(&g.iter);
            let el = native::iter_elem(it).unwrap();
            let n = self.fresh();
            let source = match &g.iter.kind {
                // `range(...)` is iterated lazily, without building a list.
                ExprKind::CallObj { func, args, kwargs } if matches!(&func.kind, ExprKind::PyBuiltin(r) if r == "range") && kwargs.is_empty() && it != Ty::Obj => {
                    let ints: Vec<String> = args.iter().map(|a| {
                        let (c, t) = self.expr(a);
                        crate::as_int(&c, t)
                    }).collect();
                    match ints.len() {
                        1 => format!("0..{}", ints[0]),
                        2 => format!("{{ let __a = {}; let __b = {}; __a..__b }}", ints[0], ints[1]),
                        _ => format!("rt::int::range({}, {}, {})?", ints[0], ints[1], ints[2]),
                    }
                }
                _ => {
                    let (src, _) = self.expr(&g.iter);
                    iter_vec(&src, it)
                }
            };
            write!(code, "for __v{n} in {source} {{ ").unwrap();
            let mut buf = String::new();
            self.assign(&g.var, format!("__v{n}"), el, 0, &mut buf);
            for st in &g.unpack {
                self.stmt(st, 0, &mut buf);
            }
            for c in &g.ifs {
                let (cc, ct) = self.expr(c);
                writeln!(buf, "if !{} {{ continue; }}", crate::truthy(&cc, ct)).unwrap();
            }
            code.push_str(&buf.replace('\n', " "));
            closers.push_str("} ");
        }
        match kind {
            CompKind::List | CompKind::Gen => {
                let x = self.expr_as(elt, t.list_elem().unwrap());
                write!(code, "{r}.append({x}); ").unwrap();
            }
            CompKind::Set => {
                let x = self.expr_as(elt, t.set_elem().unwrap());
                write!(code, "{r}.add({x}); ").unwrap();
            }
            CompKind::Dict => {
                let (k, v) = t.dict_kv().unwrap();
                let kc = self.expr_as(elt, k);
                let vc = self.expr_as(val.unwrap(), v);
                write!(code, "{{ let __k = {kc}; let __x = {vc}; {r}.set(__k, __x); }} ").unwrap();
            }
        }
        write!(code, "{closers}{r} }}").unwrap();
        code
    }

    /// `x in container` for a native container.
    pub(crate) fn contains(&mut self, x: &str, xt: Ty, c: &str, ct: Ty) -> String {
        if ct.as_record().is_some() {
            return match ops_base(xt) {
                Ty::Str if xt == Ty::Str => format!("{{ let __k = {x}; {}.has_key(&__k) }}", borrow(c)),
                Ty::Str => format!("{{ let __c = {c}; match {x} {{ Some(__k) => __c.has_key(&__k), None => false }} }}"),
                Ty::Obj => format!("rt::py::contains({}, rt::py::to_py(&{c})?)?", crate::boxed(x, xt)),
                _ => format!("{{ let _ = ({x}, {c}); false }}"),
            };
        }
        if ct == Ty::Str {
            let (x, _) = crate::unopt(x.to_string(), xt, "'in <string>' requires string as left operand, not NoneType");
            return format!("rt::string::contains(&{}, &{})", borrow(&x), borrow(c));
        }
        let e = match ct.as_comp() {
            Some(Comp::Dict(k, _)) => k,
            _ => native::iter_elem(ct).unwrap_or(Ty::never()),
        };
        if e.is_never() {
            return format!("{{ let _ = ({x}, {c}); false }}");
        }
        // `T | None` needle, elements that are never None: None is not found.
        if let (Some(inner), None) = (xt.as_opt(), e.as_opt()) {
            if e != Ty::Obj {
                let found = self.contains("__p", inner, "__c", ct);
                return format!("{{ let __c = {c}; match {x} {{ Some(__p) => {found}, None => false }} }}");
            }
        }
        // A dict key / set element of another type is never found.
        if matches!(ct.as_comp(), Some(Comp::Dict(..) | Comp::Set(_))) && e != Ty::Obj && xt != Ty::Obj && native::ty_join(e, ops_base(xt), false).is_none() {
            return format!("{{ let _ = ({x}, {c}); false }}");
        }
        let probe = convert(x, xt, e);
        let probe = borrow(&probe).to_string();
        match ct.as_comp() {
            Some(Comp::Tuple(_)) => format!("{{ let __x = {probe}; {}.iter().any(|__e| rt::coll::eq(__e, &__x)) }}", iter_vec(c, ct)),
            _ => format!("{}.contains(&{probe})", borrow(c)),
        }
    }
}

fn tuple_slice_positions(lo: &Option<Box<Expr>>, hi: &Option<Box<Expr>>, step: &Option<Box<Expr>>, len: usize) -> Vec<usize> {
    let lit = |x: &Option<Box<Expr>>| match x.as_deref().map(|e| &e.kind) {
        Some(ExprKind::Int(v)) => Some(*v),
        _ => None,
    };
    let n = len as i64;
    let step = lit(step).unwrap_or(1);
    let adj = |v: i64, a: i64, b: i64| if v < 0 { (v + n).clamp(a, b) } else { v.clamp(a, b) };
    let (mut i, stop) = if step > 0 {
        (lit(lo).map(|v| adj(v, 0, n)).unwrap_or(0), lit(hi).map(|v| adj(v, 0, n)).unwrap_or(n))
    } else {
        (lit(lo).map(|v| adj(v, -1, n - 1)).unwrap_or(n - 1), lit(hi).map(|v| adj(v, -1, n - 1)).unwrap_or(-1))
    };
    let mut out = Vec::new();
    while (step > 0 && i < stop) || (step < 0 && i > stop) {
        out.push(i as usize);
        i += step;
    }
    out
}

use std::fmt::Write;

fn ops_base(t: Ty) -> Ty {
    t.as_opt().unwrap_or(t)
}

impl FnEmitter<'_, '_> {
    /// `record["key"]` / `record.get("key")`: the field (a copy as a dict
    /// lookup if the record type lacks the key, which raises KeyError).
    /// `record.get("key"[, default])` (the default is evaluated in any case).
    pub(crate) fn record_get(&mut self, obj: &Expr, key: &str, default: Option<&Expr>, t: Ty) -> String {
        let (o, ot) = self.expr(obj);
        let o = borrow(&o).to_string();
        let fs = ot.as_opt().unwrap_or(ot).as_record().expect("record receiver");
        let (d, dt) = match default {
            Some(e) => self.expr(e),
            None => ("()".to_string(), Ty::None),
        };
        match fs.iter().position(|f| f.key == key) {
            Some(i) if !fs[i].optional => format!("{{ let _ = {d}; {} }}", convert(&format!("{o}.0.borrow().f{i}.clone()"), fs[i].ty, t)),
            Some(i) => format!("{{ let __d = {d}; match {o}.opt_f{i}() {{ Some(__x) => {}, None => {} }} }}", convert("__x", fs[i].ty, t), convert("__d", dt, t)),
            None => format!("{{ let _ = &{o}; {} }}", convert(&d, dt, t)),
        }
    }
}
