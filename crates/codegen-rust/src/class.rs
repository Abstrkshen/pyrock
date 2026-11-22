//! Code generation for natively compiled classes.
//!
//! Each class becomes a local newtype over `Rc<RefCell<..Data>>` so that
//! identity and shared mutation behave like Python objects, and so that the
//! runtime traits (printing, `==`, ordering, truthiness, hashing) can be
//! implemented for it (the orphan rule forbids impls on `rt::Ref<T>`).

use crate::{borrow, coerce, FnEmitter};
use pyfast_analysis::Plan;
use pyfast_infer::{Program, SpecKey};
use pyfast_ir::*;
use std::fmt::Write;

fn field_ident(name: &str) -> String {
    format!("f_{}", name.trim_start_matches("r#"))
}

/// Instance attributes of `c` with their known types, in declaration order.
pub(crate) fn fields(prog: &Program, c: ClassId) -> Vec<(String, Ty)> {
    let m = prog.module;
    let mut names = m.declared_fields(c);
    for (cc, f) in prog.fields.keys() {
        if *cc == c && !names.contains(f) {
            names.push(f.clone());
        }
    }
    names.into_iter().filter_map(|n| prog.fields.get(&(c, n.clone())).and_then(|s| s.known()).map(|t| (n, t))).collect()
}

fn spec_name(prog: &Program, plan: &Plan, func: FuncId, args: Vec<Ty>) -> Option<String> {
    prog.spec(&SpecKey { func, args }).filter(|s| plan.is_reachable(s.id)).map(|s| s.mangled.clone())
}

/// Struct definitions and trait impls for every class that is used.
pub(crate) fn emit_classes(prog: &Program, plan: &Plan, out: &mut String) {
    let m = prog.module;
    for cd in &m.classes {
        let c = cd.id;
        let used = prog.instantiated.contains(&c) || prog.fields.keys().any(|(k, _)| *k == c);
        if !used {
            continue;
        }
        let t = Ty::class(c, &cd.rust);
        let r = &cd.rust;
        let module_name = if cd.unit == 0 { "__main__".to_string() } else { m.units[cd.unit].name.clone() };
        let short = cd.name.rsplit('.').next().unwrap_or("");
        writeln!(out, "\n/// `class {}` from {}", cd.name, m.units[cd.unit].path.rsplit('/').next().unwrap_or("")).unwrap();
        writeln!(out, "#[derive(Default)]\npub struct {r}Data {{").unwrap();
        for (f, ft) in fields(prog, c) {
            writeln!(out, "    pub {}: Option<{}>,", field_ident(&f), ft.rust()).unwrap();
        }
        writeln!(out, "    /// Message of an exception instance (`BaseException.__init__`).\n    pub exc_msg: Option<rt::Str>,").unwrap();
        writeln!(out, "}}\n").unwrap();
        writeln!(out, "#[derive(Clone)]\npub struct {r}(pub std::rc::Rc<std::cell::RefCell<{r}Data>>);\n").unwrap();
        writeln!(out, "impl {r} {{").unwrap();
        writeln!(out, "    pub fn new() -> Self {{\n        {r}(std::rc::Rc::new(std::cell::RefCell::new({r}Data::default())))\n    }}").unwrap();
        writeln!(out, "    pub fn is(&self, o: &Self) -> bool {{\n        std::rc::Rc::ptr_eq(&self.0, &o.0)\n    }}").unwrap();
        writeln!(out, "}}\n").unwrap();
        writeln!(out, "impl Default for {r} {{\n    fn default() -> Self {{\n        Self::new()\n    }}\n}}\n").unwrap();
        if m.exception_base(c).is_some() {
            writeln!(
                out,
                "impl {r} {{\n    /// Raise this instance as a Python exception.\n    pub fn into_exc(&self) -> rt::Exc {{\n        let mut x = rt::Exc::new({:?}, rt::conv::str_of(self).to_string());\n        x.origin = Some(std::rc::Rc::new(self.clone()));\n        x\n    }}\n}}\n",
                // CPython prints `__main__` classes without the module.
                if cd.unit == 0 { short.to_string() } else { format!("{module_name}.{short}") }
            )
            .unwrap();
        }

        // repr / str
        let default_repr = format!("rt::Str::from(format!(\"<{module_name}.{short} object at {{:#x}}>\", std::rc::Rc::as_ptr(&self.0) as usize))");
        let repr = match m.find_method(c, "__repr__").and_then(|f| spec_name(prog, plan, f.func, vec![t])) {
            Some(f) => format!("{f}(self.clone()).unwrap_or_else(|e| rt::Str::from(format!(\"<repr raised {{e}}>\")))"),
            None if cd.dataclass => dataclass_repr(prog, c, short),
            None => default_repr,
        };
        let str_ = match m.find_method(c, "__str__").and_then(|f| spec_name(prog, plan, f.func, vec![t])) {
            Some(f) => format!("{f}(self.clone()).unwrap_or_else(|e| rt::Str::from(format!(\"<str raised {{e}}>\")))"),
            None if m.exception_base(c).is_some() => "self.0.borrow().exc_msg.clone().unwrap_or_else(|| rt::Str::from(\"\"))".to_string(),
            None => "self.py_repr()".to_string(),
        };
        writeln!(out, "impl rt::conv::PyFmt for {r} {{\n    fn py_repr(&self) -> rt::Str {{\n        {repr}\n    }}\n    fn py_str(&self) -> rt::Str {{\n        {str_}\n    }}\n}}\n").unwrap();

        // == (identity unless __eq__, or field-wise for dataclasses)
        let eq = match m.find_method(c, "__eq__").and_then(|f| spec_name(prog, plan, f.func, vec![t, t])) {
            Some(f) => {
                let rt_ty = prog.spec(&SpecKey { func: m.find_method(c, "__eq__").unwrap().func, args: vec![t, t] }).and_then(|s| match s.ret {
                    pyfast_infer::Slot::Known(x) => Some(x),
                    _ => None,
                });
                let res = crate::truthy("__r", rt_ty.unwrap_or(Ty::Bool));
                format!("match {f}(self.clone(), o.clone()) {{ Ok(__r) => {res}, Err(_) => false }}")
            }
            None if cd.dataclass => {
                let parts: Vec<String> = fields(prog, c).iter().map(|(f, _)| format!("rt::coll::eq(&a.{0}, &b.{0})", field_ident(f))).collect();
                if parts.is_empty() {
                    "true".into()
                } else {
                    format!("self.is(o) || {{ let (a, b) = (self.0.borrow(), o.0.borrow()); {} }}", parts.join(" && "))
                }
            }
            None => "self.is(o)".to_string(),
        };
        writeln!(out, "impl rt::coll::PyEq for {r} {{\n    fn py_eq(&self, o: &Self) -> bool {{\n        {eq}\n    }}\n}}\n").unwrap();

        // ordering
        let ord = match m.find_method(c, "__lt__").and_then(|f| spec_name(prog, plan, f.func, vec![t, t])) {
            Some(f) => {
                let rt_ty = prog.spec(&SpecKey { func: m.find_method(c, "__lt__").unwrap().func, args: vec![t, t] }).and_then(|s| match s.ret {
                    pyfast_infer::Slot::Known(x) => Some(x),
                    _ => None,
                });
                let lt = |a: &str, b: &str| format!("{{ let __r = {f}({a}.clone(), {b}.clone())?; {} }}", crate::truthy("__r", rt_ty.unwrap_or(Ty::Bool)));
                format!("if {} {{ Ok(std::cmp::Ordering::Less) }} else if {} {{ Ok(std::cmp::Ordering::Greater) }} else {{ Ok(std::cmp::Ordering::Equal) }}", lt("self", "o"), lt("o", "self"))
            }
            None => format!("Err(rt::Exc::new(\"TypeError\", \"'<' not supported between instances of '{short}' and '{short}'\"))"),
        };
        writeln!(out, "impl rt::coll::PyOrd for {r} {{\n    fn py_cmp(&self, o: &Self) -> rt::PyResult<std::cmp::Ordering> {{\n        {ord}\n    }}\n}}\n").unwrap();

        // truthiness: __bool__, then __len__, else True
        let truth = if let Some(f) = m.find_method(c, "__bool__").and_then(|f| spec_name(prog, plan, f.func, vec![t])) {
            format!("{f}(self.clone()).map(|v| rt::truthy(&v)).unwrap_or(true)")
        } else if let Some(f) = m.find_method(c, "__len__").and_then(|f| spec_name(prog, plan, f.func, vec![t])) {
            format!("{f}(self.clone()).map(|v| v != 0).unwrap_or(true)")
        } else {
            "true".to_string()
        };
        writeln!(out, "impl rt::Truthy for {r} {{\n    fn truthy(&self) -> bool {{\n        {truth}\n    }}\n}}\n").unwrap();

        // hashing (dict keys / set members): identity unless __hash__
        let hash = match m.find_method(c, "__hash__").and_then(|f| spec_name(prog, plan, f.func, vec![t])) {
            Some(f) => format!("std::hash::Hash::hash(&{f}(self.clone()).unwrap_or(0), h)"),
            None => "std::hash::Hash::hash(&(std::rc::Rc::as_ptr(&self.0) as usize), h)".to_string(),
        };
        writeln!(out, "impl PartialEq for {r} {{\n    fn eq(&self, o: &Self) -> bool {{\n        rt::coll::PyEq::py_eq(self, o)\n    }}\n}}\nimpl Eq for {r} {{}}").unwrap();
        writeln!(out, "impl std::hash::Hash for {r} {{\n    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {{\n        {hash}\n    }}\n}}").unwrap();
    }
}

/// An enum for every union of native classes, with delegating trait impls.
pub(crate) fn emit_unions(prog: &Program, out: &mut String) {
    for u in Ty::all_unions() {
        let ms = u.union_members().unwrap();
        let classes = ms.iter().all(|m| m.class_id().is_some());
        if classes && !ms.iter().all(|m| m.class_id().is_some_and(|c| prog.instantiated.contains(&c) || prog.fields.keys().any(|(k, _)| *k == c))) {
            continue;
        }
        if !classes && !ms.iter().all(|m| m.as_closure().is_some()) {
            continue;
        }
        let r = u.rust();
        let vnames: Vec<String> = ms.iter().map(|m| m.variant_name()).collect();
        let vs: Vec<&str> = vnames.iter().map(|s| s.as_str()).collect();
        writeln!(out, "\n/// `{}`\n#[derive(Clone)]\npub enum {r} {{", u.py_name()).unwrap();
        for (v, m) in vs.iter().zip(&ms) {
            writeln!(out, "    {v}({}),", m.rust()).unwrap();
        }
        writeln!(out, "}}\n").unwrap();
        let arms = |body: &str| vs.iter().map(|v| format!("Self::{v}(x) => {body}")).collect::<Vec<_>>().join(", ");
        let pair_arms = |body: &str, other: &str| {
            let mut a: Vec<String> = vs.iter().map(|v| format!("(Self::{v}(a), Self::{v}(b)) => {body}")).collect();
            a.push(format!("_ => {other}"));
            a.join(", ")
        };
        writeln!(out, "impl Default for {r} {{\n    fn default() -> Self {{\n        Self::{}(Default::default())\n    }}\n}}", vs[0]).unwrap();
        if classes {
            writeln!(out, "impl {r} {{\n    pub fn is(&self, o: &Self) -> bool {{\n        match (self, o) {{ {} }}\n    }}\n}}", pair_arms("a.is(b)", "false")).unwrap();
        }
        writeln!(out, "impl rt::conv::PyFmt for {r} {{\n    fn py_repr(&self) -> rt::Str {{\n        match self {{ {} }}\n    }}\n    fn py_str(&self) -> rt::Str {{\n        match self {{ {} }}\n    }}\n}}", arms("rt::conv::repr_of(x)"), arms("rt::conv::str_of(x)")).unwrap();
        writeln!(out, "impl rt::coll::PyEq for {r} {{\n    fn py_eq(&self, o: &Self) -> bool {{\n        match (self, o) {{ {} }}\n    }}\n}}", pair_arms("rt::coll::PyEq::py_eq(a, b)", "false")).unwrap();
        if classes {
            writeln!(out, "impl rt::coll::PyOrd for {r} {{\n    fn py_cmp(&self, o: &Self) -> rt::PyResult<std::cmp::Ordering> {{\n        match (self, o) {{ {} }}\n    }}\n}}", pair_arms("rt::coll::PyOrd::py_cmp(a, b)", "Err(rt::Exc::new(\"TypeError\", \"'<' not supported between instances of different classes\"))")).unwrap();
        }
        writeln!(out, "impl rt::Truthy for {r} {{\n    fn truthy(&self) -> bool {{\n        match self {{ {} }}\n    }}\n}}", arms("rt::Truthy::truthy(x)")).unwrap();
        if classes {
            writeln!(out, "impl PartialEq for {r} {{\n    fn eq(&self, o: &Self) -> bool {{\n        rt::coll::PyEq::py_eq(self, o)\n    }}\n}}\nimpl Eq for {r} {{}}").unwrap();
            writeln!(out, "impl std::hash::Hash for {r} {{\n    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {{\n        match self {{ {} }}\n    }}\n}}", arms("std::hash::Hash::hash(x, h)")).unwrap();
        }
    }
}

fn dataclass_repr(prog: &Program, c: ClassId, short: &str) -> String {
    let parts: Vec<String> = fields(prog, c)
        .iter()
        .map(|(f, _)| format!("format!(\"{f}={{}}\", d.{}.as_ref().map(|v| rt::conv::repr_of(v)).unwrap_or_else(|| rt::Str::from(\"?\")))", field_ident(f)))
        .collect();
    format!("{{ let d = self.0.borrow(); rt::Str::from(format!(\"{short}({{}})\", [{}].join(\", \"))) }}", parts.join(", "))
}

impl FnEmitter<'_, '_> {
    fn class_ty(&self, c: ClassId) -> Ty {
        Ty::class(c, &self.prog.module.class(c).rust)
    }

    /// Call specialization `func` with receiver code(s) and bound arguments.
    pub(crate) fn call_bound(&mut self, func: FuncId, lead: Vec<(String, Ty)>, bound: &[Expr], order: &[usize]) -> String {
        let mut vals: Vec<(String, Ty)> = bound.iter().map(|a| self.expr(a)).collect();
        let key = SpecKey { func, args: lead.iter().map(|l| l.1).chain(vals.iter().map(|v| v.1)).collect() };
        let callee = self.prog.spec(&key).expect("missing method specialization").mangled.clone();
        let mut lets = String::new();
        for &i in order {
            let tmp = self.fresh();
            write!(lets, "let {tmp} = {}; ", vals[i].0).unwrap();
            vals[i].0 = tmp;
        }
        let args: Vec<String> = lead.into_iter().map(|l| l.0).chain(vals.into_iter().map(|v| v.0)).collect();
        if lets.is_empty() {
            format!("{callee}({})?", args.join(", "))
        } else {
            format!("{{ {lets}{callee}({})? }}", args.join(", "))
        }
    }

    /// `Point(...)`
    pub(crate) fn new_instance(&mut self, e: &Expr, c: ClassId, args: &[Expr], kwargs: &[(String, Expr)]) -> String {
        let t = self.class_ty(c);
        let rust = t.rust();
        match self.prog.module.find_method(c, "__init__").cloned() {
            Some(m) if self.prog.module.exception_base(c).is_some() && kwargs.is_empty() => {
                // BaseException.__new__ keeps the arguments: str(e) is '' for none,
                // str(a) for one, the tuple's repr for several -- unless __init__
                // calls super().__init__ with another message.
                let (bound, _) = self.prog.bound_args(m.func, true, args, kwargs, e.span).expect("binding checked by inference");
                let vals: Vec<(String, Ty)> = bound.iter().map(|b| self.expr(b)).collect();
                let key = SpecKey { func: m.func, args: std::iter::once(t).chain(vals.iter().map(|v| v.1)).collect() };
                let callee = self.prog.spec(&key).expect("init spec").mangled.clone();
                let mut lets = String::new();
                for (i, (c, _)) in vals.iter().enumerate() {
                    write!(lets, "let __a{i} = {c}; ").unwrap();
                }
                let read = |i: usize| if vals[i].1.is_clone() { format!("__a{i}.clone()") } else { format!("__a{i}") };
                let n = args.len();
                let msg = match n {
                    0 => "rt::Str::from(\"\")".to_string(),
                    1 => crate::to_str(&read(0), vals[0].1),
                    _ => {
                        let parts: Vec<String> = (0..n).map(|i| format!("rt::conv::repr_of(&{}).to_string()", read(i))).collect();
                        format!("rt::Str::from(format!(\"({{}})\", [{}].join(\", \")))", parts.join(", "))
                    }
                };
                let call_args: Vec<String> = (0..vals.len()).map(|i| format!("__a{i}")).collect();
                format!(
                    "{{ let __o = {rust}::new(); {lets}__o.0.borrow_mut().exc_msg = Some({msg}); let _ = {callee}(__o.clone(){}{})?; __o }}",
                    if call_args.is_empty() { "" } else { ", " },
                    call_args.join(", ")
                )
            }
            Some(m) => {
                let (bound, order) = self.prog.bound_args(m.func, true, args, kwargs, e.span).expect("binding checked by inference");
                let init = self.call_bound(m.func, vec![("__o.clone()".into(), t)], &bound, &order);
                format!("{{ let __o = {rust}::new(); let _ = {init}; __o }}")
            }
            None => {
                if self.prog.module.exception_base(c).is_some() && !args.is_empty() {
                    // Exception subclass without __init__: the message is str(args[0]).
                    let (a, at) = self.expr(&args[0]);
                    return format!("{{ let __o = {rust}::new(); __o.0.borrow_mut().exc_msg = Some({}); __o }}", crate::to_str(&a, at));
                }
                format!("{rust}::new()")
            }
        }
    }

    /// A method call on a native instance or class object.
    pub(crate) fn method_call(&mut self, e: &Expr, obj: &Expr, func: FuncId, kind: MethodKind, args: &[Expr], kwargs: &[(String, Expr)]) -> String {
        let (o, ot) = self.expr(obj);
        self.method_call_on(e, o, ot, func, kind, args, kwargs)
    }

    /// `recv.name(...)` on a union: dispatch per variant.
    pub(crate) fn union_method(&mut self, e: &Expr, obj: &Expr, name: &str, args: &[Expr], kwargs: &[(String, Expr)], t: Ty) -> String {
        let (o, ot) = self.expr(obj);
        let ur = ot.rust();
        let mut arms = Vec::new();
        for m in ot.union_members().unwrap() {
            let c = m.class_id().unwrap();
            let meth = self.prog.module.find_method(c, name).cloned().unwrap();
            let rt = self.prog.method_type(self.spec, m, meth.func, meth.kind, args, kwargs).unwrap_or(t);
            let call = self.method_call_on(e, "__m".into(), m, meth.func, meth.kind, args, kwargs);
            arms.push(format!("{ur}::{}(__m) => {}", m.variant_name(), crate::coll::convert(&call, rt, t)));
        }
        format!("{{ let __u = {o}; match __u {{ {} }} }}", arms.join(", "))
    }

    pub(crate) fn method_call_on(&mut self, e: &Expr, o: String, ot: Ty, func: FuncId, kind: MethodKind, args: &[Expr], kwargs: &[(String, Expr)]) -> String {
        if func == usize::MAX {
            // super().__init__(msg) in an exception subclass.
            let (msg, mt) = self.expr(&args[0]);
            return format!("{{ let __m = {}; {}.0.borrow_mut().exc_msg = Some(__m); }}", crate::to_str(&msg, mt), borrow(&o));
        }
        match kind {
            MethodKind::Static => {
                let (bound, order) = self.prog.bound_args(func, false, args, kwargs, e.span).unwrap();
                format!("{{ let _ = {o}; {} }}", self.call_bound(func, vec![], &bound, &order))
            }
            MethodKind::Class => {
                let c = ot.class_id().or(match ot.as_comp() {
                    Some(Comp::Type(c, _)) => Some(c),
                    _ => None,
                });
                let tt = Ty::class_type(c.unwrap(), &self.prog.module.class(c.unwrap()).rust);
                let (bound, order) = self.prog.bound_args(func, true, args, kwargs, e.span).unwrap();
                format!("{{ let _ = {o}; {} }}", self.call_bound(func, vec![("()".into(), tt)], &bound, &order))
            }
            _ => {
                let (bound, order) = self.prog.bound_args(func, true, args, kwargs, e.span).unwrap();
                self.call_bound(func, vec![(o, ot)], &bound, &order)
            }
        }
    }

    /// Dunder `name` on a native instance with already-evaluated operands.
    pub(crate) fn dunder(&mut self, recv: (String, Ty), name: &str, others: Vec<(String, Ty)>) -> Option<String> {
        if let Some(ms) = recv.1.union_members() {
            if !ms.iter().all(|m| self.prog.module.find_method(m.class_id().unwrap(), name).is_some()) {
                return None;
            }
            // Evaluate the other operands once, then dispatch on the variant.
            let mut lets = String::new();
            let mut tmps = Vec::new();
            for (i, (c, t)) in others.iter().enumerate() {
                write!(lets, "let __d{i} = {c}; ").unwrap();
                tmps.push((format!("__d{i}"), *t));
            }
            let results: Vec<(String, Ty)> = ms
                .iter()
                .map(|m| {
                    let c = m.class_id().unwrap();
                    let f = self.prog.module.find_method(c, name).unwrap().func;
                    let key = SpecKey { func: f, args: std::iter::once(*m).chain(tmps.iter().map(|o| o.1)).collect() };
                    let s = self.prog.spec(&key);
                    let rt = s.and_then(|s| match s.ret {
                        pyfast_infer::Slot::Known(t) => Some(t),
                        pyfast_infer::Slot::Boxed(..) => Some(Ty::Obj),
                        _ => None,
                    });
                    let args: Vec<String> = std::iter::once("__m".to_string()).chain(tmps.iter().map(|o| if o.1.is_clone() { format!("{}.clone()", o.0) } else { o.0.clone() })).collect();
                    (format!("{}({})?", s.map(|s| s.mangled.clone()).unwrap_or_default(), args.join(", ")), rt.unwrap_or(Ty::None))
                })
                .collect();
            let joined = results.iter().skip(1).try_fold(results[0].1, |a, r| pyfast_infer::native::ty_join(a, r.1, true)).unwrap_or(Ty::Obj);
            let ur = recv.1.rust();
            let arms: Vec<String> = ms.iter().zip(&results).map(|(m, (code, rt))| format!("{ur}::{}(__m) => {}", m.variant_name(), crate::coll::convert(code, *rt, joined))).collect();
            return Some(format!("{{ let __u = {}; {lets}match __u {{ {} }} }}", recv.0, arms.join(", ")));
        }
        let c = recv.1.class_id()?;
        let m = self.prog.module.find_method(c, name)?;
        let key = SpecKey { func: m.func, args: std::iter::once(recv.1).chain(others.iter().map(|o| o.1)).collect() };
        let callee = self.prog.spec(&key)?.mangled.clone();
        let args: Vec<String> = std::iter::once(recv.0).chain(others.into_iter().map(|o| o.0)).collect();
        Some(format!("{callee}({})?", args.join(", ")))
    }

    /// `obj.name` on a native instance or class object.
    pub(crate) fn attr(&mut self, obj: &Expr, name: &str, t: Ty) -> String {
        let (o, ot) = self.expr(obj);
        if let Some(ms) = ot.union_members() {
            let ur = ot.rust();
            let mut arms = Vec::new();
            for m in ms {
                let mt = self.prog.attr_type(self.spec, m, name).unwrap_or(t);
                let code = self.attr_code("__m".into(), m, name, mt);
                arms.push(format!("{ur}::{}(__m) => {}", m.variant_name(), crate::coll::convert(&code, mt, t)));
            }
            return format!("{{ let __u = {o}; match __u {{ {} }} }}", arms.join(", "));
        }
        self.attr_code(o, ot, name, t)
    }

    fn attr_code(&mut self, o: String, ot: Ty, name: &str, t: Ty) -> String {
        let m = self.prog.module;
        let class_var = |c: ClassId| -> Option<(String, Ty)> {
            let owner = m.find_class_var(c, name)?;
            let cd = m.class(owner);
            let short = cd.name.rsplit('.').next().unwrap_or("").to_string();
            let key = (cd.unit, format!("{short}.{name}"));
            let gt = self.prog.globals.get(&key).and_then(|s| s.known())?;
            Some((format!("rt::gget(&{}, {:?})?", crate::gname(key.0, &key.1), key.1), gt))
        };
        if let Some(Comp::Type(c, _)) = ot.as_comp() {
            let (code, gt) = class_var(c).expect("class attribute checked by inference");
            return format!("{{ let _ = {o}; {} }}", coerce(code, gt, t));
        }
        let c = ot.class_id().unwrap();
        if let Some(meth) = m.find_method(c, name).cloned() {
            if meth.kind == MethodKind::Property {
                let key = SpecKey { func: meth.func, args: vec![ot] };
                let callee = self.prog.spec(&key).expect("property getter").mangled.clone();
                return format!("{callee}({o})?");
            }
        }
        let field = self.prog.fields.get(&(c, name.to_string())).and_then(|s| s.known());
        let short = m.class(c).name.rsplit('.').next().unwrap_or("").to_string();
        if t.is_never() {
            return format!("{{ let _ = {o}; let __n: rt::Never = return Err(rt::Exc::new(\"AttributeError\", \"'{short}' object has no attribute '{name}'\")); __n }}");
        }
        let missing = match class_var(c) {
            Some((code, gt)) => coerce(code, gt, t),
            None => format!("return Err(rt::Exc::new(\"AttributeError\", \"'{short}' object has no attribute '{name}'\"))"),
        };
        match field {
            Some(ft) => {
                let got = coerce("__x".into(), ft, t);
                format!("{{ let __v = {}.0.borrow().{}.clone(); match __v {{ Some(__x) => {got}, None => {missing} }} }}", borrow(&o), field_ident(name))
            }
            None => format!("{{ let _ = {o}; {missing} }}"),
        }
    }

    /// `obj.name = value` on a native instance.
    pub(crate) fn set_attr(&mut self, obj: &Expr, name: &str, value: &Expr, ind: &str) -> String {
        let ot = self.ty(obj);
        if let Some(ms) = ot.union_members() {
            let (v, vt) = self.expr(value);
            let o = self.expr(obj).0;
            let ur = ot.rust();
            let arms: Vec<String> = ms
                .iter()
                .map(|m| {
                    let c = m.class_id().unwrap();
                    let ft = self.prog.fields.get(&(c, name.to_string())).and_then(|s| s.known()).unwrap_or(vt);
                    format!("{ur}::{}(__m) => {{ __m.0.borrow_mut().{} = Some({}); }}", m.variant_name(), field_ident(name), crate::coll::convert("__v.clone()", vt, ft))
                })
                .collect();
            return format!("{ind}{{\n{ind}    let __v = {v};\n{ind}    match {o} {{ {} }}\n{ind}}}", arms.join(", "));
        }
        let c = ot.class_id().unwrap();
        let m = self.prog.module;
        let setter = m.mro(c).into_iter().find_map(|k| m.class(k).methods.iter().find(|mm| mm.name == name && mm.kind == MethodKind::Setter).cloned());
        if let Some(st) = setter {
            let (v, vt) = self.expr(value);
            let o = self.expr(obj).0;
            let key = SpecKey { func: st.func, args: vec![ot, vt] };
            let callee = self.prog.spec(&key).expect("property setter").mangled.clone();
            return format!("{ind}{{\n{ind}    let __v = {v};\n{ind}    {callee}({o}, __v)?;\n{ind}}}");
        }
        let ft = self.prog.fields.get(&(c, name.to_string())).and_then(|s| s.known()).unwrap_or_else(|| self.ty(value));
        // CPython evaluates the value before the target object.
        let v = self.expr_as(value, ft);
        let o = self.expr(obj).0;
        format!("{ind}{{\n{ind}    let __v = {v};\n{ind}    {}.0.borrow_mut().{} = Some(__v);\n{ind}}}", borrow(&o), field_ident(name))
    }
}
