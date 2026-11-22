//! Code generation for function values: nested functions, lambdas,
//! closures (shared cells) and calls through function values.

use crate::{borrow, FnEmitter};
use pyfast_infer::spec::env_offset;
use pyfast_infer::SpecKey;
use pyfast_ir::*;

impl FnEmitter<'_, '_> {
    /// Whether local `n` lives in a closure cell (captured, or captured by a nested function).
    pub(crate) fn is_cell(&self, n: &str) -> bool {
        self.func.cells.iter().any(|c| c == n) || self.func.captures.iter().any(|c| c == n)
    }

    /// `lambda` / nested `def` value: the environment holds clones of the cells.
    pub(crate) fn make_closure(&mut self, captures: &[String], defaults: &[Expr]) -> String {
        if captures.is_empty() && defaults.is_empty() {
            return "rt::Env(())".into();
        }
        let mut parts: Vec<String> = captures.iter().map(|c| format!("{}.clone()", crate::rid(c))).collect();
        for d in defaults {
            let (c, _) = self.expr(d);
            parts.push(format!("{{ let __c = rt::cell(); rt::cset(&__c, {c}); __c }}"));
        }
        format!("rt::Env(({},))", parts.join(", "))
    }

    /// `f(args)` where `f` is a function value.
    pub(crate) fn closure_call(&mut self, e: &Expr, func: &Expr, args: &[Expr], kwargs: &[(String, Expr)]) -> String {
        let (fv, ft) = self.expr(func);
        if let Some(ms) = ft.union_members() {
            // A union of function values: dispatch on the variant.
            let t = self.ty(e);
            let ur = ft.rust();
            let mut arms = Vec::new();
            for m in ms {
                let (fid, caps) = m.as_closure().unwrap();
                let (bound, _) = self.prog.bound_args(fid, false, args, kwargs, e.span).expect("binding checked by inference");
                let env = env_offset(&self.prog.module.funcs[fid]) == 1;
                // Defaults stored in this variant's environment are read from it.
                let vals: Vec<(String, Ty)> = bound
                    .iter()
                    .map(|a| match a.kind {
                        ExprKind::EnvDefault(i) => (format!("rt::cget(&__fv.0.{i}, \"default\")?"), caps[i]),
                        _ => self.expr(a),
                    })
                    .collect();
                let key: Vec<Ty> = if env { std::iter::once(m).chain(vals.iter().map(|v| v.1)).collect() } else { vals.iter().map(|v| v.1).collect() };
                let spec = self.prog.spec(&SpecKey { func: fid, args: key }).expect("function value specialization");
                let rt = match spec.ret {
                    pyfast_infer::Slot::Known(t) => t,
                    pyfast_infer::Slot::Boxed(..) => Ty::Obj,
                    _ => Ty::None,
                };
                let mut call_args: Vec<String> = if env { vec!["__fv.clone()".into()] } else { vec![] };
                call_args.extend(vals.into_iter().map(|v| v.0));
                let call = format!("{}({})?", spec.mangled, call_args.join(", "));
                arms.push(format!("{ur}::{}(__fv) => {}", m.variant_name(), crate::coll::convert(&call, rt, t)));
            }
            return format!("{{ let __u = {fv}; match __u {{ {} }} }}", arms.join(", "));
        }
        let (fid, _) = ft.as_closure().expect("closure call on a function value");
        let (bound, order) = self.prog.bound_args(fid, false, args, kwargs, e.span).expect("binding checked by inference");
        let bound = pyfast_infer::spec::env_defaults(bound, func);
        if env_offset(&self.prog.module.funcs[fid]) == 1 {
            self.call_bound(fid, vec![(fv, ft)], &bound, &order)
        } else {
            format!("{{ let _ = {fv}; {} }}", self.call_bound(fid, vec![], &bound, &order))
        }
    }

    /// A Rust closure `|a0, a1, ...| -> PyResult<R>` calling function value
    /// `fv` (bound to `__f`, which the caller must define) with positional
    /// arguments of types `arg_tys` (missing parameters take their defaults).
    pub(crate) fn value_callable(&mut self, ft: Ty, arg_tys: &[Ty]) -> (String, Ty) {
        let (fid, _) = ft.as_closure().unwrap();
        let f = &self.prog.module.funcs[fid];
        let env = env_offset(f) == 1;
        let defaults: Vec<Expr> = f.sig.defaults.iter().skip(arg_tys.len()).flatten().cloned().collect();
        let (_, caps) = ft.as_closure().unwrap();
        let dvals: Vec<(String, Ty)> = defaults
            .iter()
            .map(|d| match d.kind {
                ExprKind::EnvDefault(i) => (format!("rt::cget(&__f.0.{i}, \"default\")?"), caps[i]),
                _ => self.expr(d),
            })
            .collect();
        let mut key_args: Vec<Ty> = if env { vec![ft] } else { vec![] };
        key_args.extend(arg_tys);
        key_args.extend(dvals.iter().map(|d| d.1));
        let spec = self.prog.spec(&SpecKey { func: fid, args: key_args }).expect("function value specialization");
        let ret = match spec.ret {
            pyfast_infer::Slot::Known(t) => t,
            pyfast_infer::Slot::Boxed(..) => Ty::Obj,
            _ => Ty::None,
        };
        let name = spec.mangled.clone();
        let params: Vec<String> = arg_tys.iter().enumerate().map(|(i, t)| format!("__a{i}: {}", t.rust())).collect();
        let mut call_args: Vec<String> = if env { vec!["__f.clone()".into()] } else { vec![] };
        call_args.extend((0..arg_tys.len()).map(|i| format!("__a{i}")));
        call_args.extend(dvals.into_iter().map(|d| d.0));
        (format!("|{}| -> PyResult<{}> {{ {name}({}) }}", params.join(", "), ret.rust(), call_args.join(", ")), ret)
    }

    /// `sorted(xs, key=f, reverse=r)`, `min/max(xs, key=f)`, `map`, `filter`.
    pub(crate) fn higher_order(&mut self, name: &str, args: &[Expr], kwargs: &[(String, Expr)], t: Ty) -> String {
        let kw = |k: &str| kwargs.iter().find(|(n, _)| n == k).map(|(_, v)| v);
        match name {
            "sorted" | "min" | "max" => {
                let (xs, xt) = self.expr(&args[0]);
                let el = pyfast_infer::native::iter_elem(xt).unwrap();
                let (fv, ft) = self.expr(kw("key").unwrap());
                let rev = match kw("reverse") {
                    Some(r) => {
                        let (c, rt) = self.expr(r);
                        crate::truthy(&c, rt)
                    }
                    None => "false".into(),
                };
                let items = crate::coll::iter_vec(&xs, xt);
                if el.is_never() {
                    return if name == "sorted" { format!("{{ let _ = ({fv}, {items}); rt::List::<rt::Never>::new() }}") } else { format!("{{ let _ = {fv}; rt::coll::min_of({items}, {})? }}", name == "max") };
                }
                let (callable, _) = self.value_callable(ft, &[el]);
                let key = format!("|__x: &{}| (({callable}))(__x.clone())", el.rust());
                match name {
                    "sorted" => format!("{{ let __f = {fv}; rt::coll::sorted_by_key({items}, {key}, {rev})? }}"),
                    _ => format!("{{ let __f = {fv}; rt::coll::min_by_key({items}, {key}, {})? }}", name == "max"),
                }
            }
            "map" => {
                let (fv, ft) = self.expr(&args[0]);
                let its: Vec<(String, Ty)> = args[1..].iter().map(|a| self.expr(a)).collect();
                let els: Vec<Ty> = its.iter().map(|(_, t)| pyfast_infer::native::iter_elem(*t).unwrap()).collect();
                let (callable, rt) = self.value_callable(ft, &els);
                let out_el = t.list_elem().unwrap_or(rt);
                let lets: String = its.iter().enumerate().map(|(i, (c, it))| format!("let mut __i{i} = {}.into_iter(); ", crate::coll::iter_vec(c, *it))).collect();
                let nexts: Vec<String> = (0..its.len()).map(|i| format!("__i{i}.next()")).collect();
                let pats: Vec<String> = (0..its.len()).map(|i| format!("Some(__v{i})")).collect();
                let vals: Vec<String> = (0..its.len()).map(|i| format!("__v{i}")).collect();
                format!(
                    "{{ let __f = {fv}; let __c = {callable}; {lets}let mut __out = Vec::new(); while let ({},) = ({},) {{ __out.push({}); }} rt::List::<{}>::from_vec(__out) }}",
                    pats.join(", "),
                    nexts.join(", "),
                    crate::coll::convert(&format!("__c({})?", vals.join(", ")), rt, out_el),
                    out_el.rust()
                )
            }
            _ => {
                let (fv, ft) = self.expr(&args[0]);
                let (xs, xt) = self.expr(&args[1]);
                let el = pyfast_infer::native::iter_elem(xt).unwrap();
                let (callable, rt) = self.value_callable(ft, &[el]);
                format!(
                    "{{ let __f = {fv}; let __c = {callable}; let mut __out = Vec::new(); for __x in {} {{ let __r = __c(__x.clone())?; if {} {{ __out.push(__x); }} }} rt::List::<{}>::from_vec(__out) }}",
                    crate::coll::iter_vec(&xs, xt),
                    crate::truthy("__r", rt),
                    el.rust()
                )
            }
        }
    }

    /// `xs.sort(key=f, reverse=r)` on a native list.
    pub(crate) fn sort_by_key(&mut self, recv: &str, el: Ty, key: &Expr, rev: String) -> String {
        let (fv, ft) = self.expr(key);
        if el.is_never() {
            return format!("{{ let _ = {fv}; }}");
        }
        let (callable, _) = self.value_callable(ft, &[el]);
        format!("{{ let __f = {fv}; {}.sort_by_key(|__x: &{}| (({callable}))(__x.clone()), {rev})?; }}", borrow(recv), el.rust())
    }
}

/// `box_<type>(f)`: wrap a native function value as a Python callable that
/// binds Python arguments and calls the all-`object` specialization.
pub(crate) fn emit_boxers(prog: &pyfast_infer::Program, out: &mut String) {
    use std::fmt::Write;
    for &t in &prog.boxed_fns {
        let (fid, caps) = t.as_closure().unwrap();
        let f = &prog.module.funcs[fid];
        let env = env_offset(f) == 1;
        let mut key: Vec<Ty> = if env { vec![t] } else { vec![] };
        key.extend(f.sig.names.iter().map(|_| Ty::Obj));
        if f.sig.vararg.is_some() {
            key.push(Ty::tuple_var(Ty::Obj));
        }
        if f.sig.kwarg.is_some() {
            key.push(Ty::dict(Ty::Str, Ty::Obj));
        }
        let Some(spec) = prog.spec(&SpecKey { func: fid, args: key }) else { continue };
        let ret = match spec.ret {
            pyfast_infer::Slot::Known(r) => r,
            pyfast_infer::Slot::Boxed(..) => Ty::Obj,
            _ => Ty::None,
        };
        let short = f.name.rsplit('.').next().unwrap_or(&f.name).to_string();
        let params: Vec<String> = f.sig.names.iter().map(|n| format!("{n:?}")).collect();
        let mut body = String::new();
        writeln!(body, "        let __b = rt::py::bind_py_args({short:?}, &[{}], __args, __kwargs)?;", params.join(", ")).unwrap();
        let mut call_args: Vec<String> = if env { vec!["__f.clone()".into()] } else { vec![] };
        for (i, n) in f.sig.names.iter().enumerate() {
            let missing = match f.sig.defaults.get(i).cloned().flatten() {
                Some(Expr { kind: ExprKind::EnvDefault(slot), .. }) => crate::boxed(&format!("rt::cget(&__f.0.{slot}, \"default\")?"), caps[slot]),
                Some(d) => literal_obj(&d),
                None => format!("return Err(rt::py::missing_arg({short:?}, {n:?}))"),
            };
            call_args.push(format!("match __b[{i}].clone() {{ Some(__v) => __v, None => {missing} }}"));
        }
        if f.sig.vararg.is_some() {
            call_args.push("rt::Tup::from_vec(vec![])".into());
        }
        if f.sig.kwarg.is_some() {
            call_args.push("rt::Dict::new()".into());
        }
        writeln!(body, "        let __r = {}({})?;", spec.mangled, call_args.join(", ")).unwrap();
        writeln!(body, "        Ok({})", crate::boxed("__r", ret)).unwrap();
        writeln!(
            out,
            "\n/// `{}` as a Python callable.\nfn box_{}(__f: {}) -> rt::py::Obj {{\n    rt::py::native_fn({short:?}, move |__args, __kwargs| {{\n{body}    }})\n}}",
            f.name,
            t.mangle(),
            t.rust()
        )
        .unwrap();
    }
}

/// A literal default as a Python object.
fn literal_obj(d: &Expr) -> String {
    match &d.kind {
        ExprKind::Int(v) => format!("rt::py::int({v}_i64)"),
        ExprKind::Float(v) => format!("rt::py::float({v:?}_f64)"),
        ExprKind::Str(v) => format!("rt::py::str_({v:?})"),
        ExprKind::Bool(b) => format!("rt::py::bool_({b})"),
        ExprKind::NoneLit => "rt::py::none()".into(),
        _ => "rt::py::none()".into(),
    }
}
