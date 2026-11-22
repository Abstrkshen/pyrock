//! Eligibility planning (design section 4): decides, per specialization,
//! whether code is fully native or crosses into CPython, records every
//! boundary with its reason, and summarises effects for the explain report.
//!
//! In `native-only` mode each boundary becomes a compile error.

use pyfast_infer::{CallClass, Program, Slot, Spec};
use pyfast_ir::*;
use std::collections::{BTreeSet, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// No Python objects anywhere in the specialization.
    Native,
    /// Native code that calls into CPython at listed boundaries.
    Bridge,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct PlanEntry {
    pub spec_id: usize,
    pub function: String,
    pub signature: String,
    pub rust_name: String,
    pub tier: Tier,
    pub recursive: bool,
    pub effects: Vec<&'static str>,
    pub span: Span,
}

/// One place where native code meets CPython.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Boundary {
    pub code: &'static str,
    pub span: Span,
    pub function: String,
    pub what: String,
    pub reason: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Plan {
    pub entries: Vec<PlanEntry>,
    /// Functions defined but never reached from the module body.
    pub unreachable: Vec<String>,
    pub boundaries: Vec<Boundary>,
    /// Whether the binary must embed CPython.
    pub uses_python: bool,
    /// Rust / C modules actually called.
    pub externs_used: BTreeSet<usize>,
    /// Specializations reachable from the module body under the final types
    /// (widening can leave earlier, now-unused specializations behind).
    pub reachable: BTreeSet<usize>,
}

impl Plan {
    pub fn is_reachable(&self, spec_id: usize) -> bool {
        self.reachable.contains(&spec_id)
    }

    pub fn is_recursive(&self, spec_id: usize) -> bool {
        self.entries.iter().any(|e| e.spec_id == spec_id && e.recursive)
    }

    /// (native, bridge) specialization counts, excluding the entry module body.
    pub fn counts(&self) -> (usize, usize) {
        let n = self.entries.iter().skip(1).filter(|e| e.tier == Tier::Native).count();
        (n, self.entries.len() - 1 - n)
    }
}

fn spec_calls(prog: &Program, s: &Spec) -> BTreeSet<usize> {
    let f = &prog.module.funcs[s.key.func];
    let mut out = BTreeSet::new();
    walk_block(&f.body, &mut |e| {
        if let ExprKind::Call { callee: Callee::Func(fid), args, .. } = &e.kind {
            let tys: Option<Vec<Ty>> = args.iter().map(|a| prog.type_of(s, a)).collect();
            if let Some(args) = tys {
                if let Some(c) = prog.spec(&pyfast_infer::SpecKey { func: *fid, args }) {
                    out.insert(c.id);
                }
            }
        }
        // Methods, constructors and dunders of native classes.
        for key in prog.class_calls(s, e) {
            if let Some(c) = prog.spec(&key) {
                out.insert(c.id);
            }
        }
    });
    walk_stmts(&f.body, &mut |st| {
        if let StmtKind::Import(ImportAction::Unit(fid)) = &st.kind {
            if let Some(c) = prog.spec(&pyfast_infer::SpecKey { func: *fid, args: vec![] }) {
                out.insert(c.id);
            }
        }
        for key in prog.class_stmt_calls(s, st) {
            if let Some(c) = prog.spec(&key) {
                out.insert(c.id);
            }
        }
    });
    // Trait impls of instantiated classes call their dunders.
    if s.id == 0 {
        for &c in &prog.instantiated {
            let t = Ty::class(c, &prog.module.class(c).rust);
            for (d, args) in [("__repr__", vec![t]), ("__str__", vec![t]), ("__len__", vec![t]), ("__bool__", vec![t]), ("__hash__", vec![t]), ("__eq__", vec![t, t]), ("__lt__", vec![t, t])] {
                if let Some(m) = prog.module.find_method(c, d) {
                    if let Some(sp) = prog.spec(&pyfast_infer::SpecKey { func: m.func, args }) {
                        out.insert(sp.id);
                    }
                }
            }
        }
    }
    out
}

/// Call edges from each specialization to the specializations it calls.
pub fn call_graph(prog: &Program) -> Vec<BTreeSet<usize>> {
    prog.specs.iter().map(|s| spec_calls(prog, s)).collect()
}

pub fn plan(prog: &Program) -> (Plan, Vec<Diagnostic>) {
    let g = call_graph(prog);
    let n = prog.specs.len();
    let mut reachable = BTreeSet::new();
    let mut stack = vec![0usize];
    while let Some(x) = stack.pop() {
        if reachable.insert(x) {
            stack.extend(g[x].iter().copied());
        }
    }
    let reaches = |from: usize, to: usize| -> bool {
        let mut seen = HashSet::new();
        let mut stack: Vec<usize> = g[from].iter().copied().collect();
        while let Some(x) = stack.pop() {
            if x == to {
                return true;
            }
            if seen.insert(x) {
                stack.extend(g[x].iter().copied());
            }
        }
        false
    };

    let mut boundaries = Vec::new();
    let mut per_spec_bridge = vec![false; n];
    let mut externs_used = BTreeSet::new();
    let mut eff: Vec<BTreeSet<&'static str>> = Vec::with_capacity(n);
    for s in &prog.specs {
        if !reachable.contains(&s.id) {
            eff.push(BTreeSet::new());
            continue;
        }
        let f = &prog.module.funcs[s.key.func];
        let mut e = BTreeSet::new();
        walk_block(&f.body, &mut |x| local_effects(prog, s, x, &mut e));
        walk_block(&f.body, &mut |x| {
            if let ExprKind::ExternCall { ext, .. } = x.kind {
                externs_used.insert(ext);
            }
        });
        let before = boundaries.len();
        let label = pyfast_infer::spec_label(prog, s);
        find_boundaries(prog, s, &label, &mut boundaries);
        if boundaries.len() > before {
            per_spec_bridge[s.id] = true;
            e.insert("calls_python");
        }
        eff.push(e);
    }
    // Effects flow from callees to callers.
    loop {
        let mut changed = false;
        for i in 0..n {
            for &c in &g[i].clone() {
                let add: Vec<_> = eff[c].iter().copied().collect();
                for a in add {
                    changed |= eff[i].insert(a);
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut seen = HashSet::new();
    boundaries.retain(|b| seen.insert((b.span, b.what.clone())));
    boundaries.sort_by_key(|b| (b.span.unit, b.span.line, b.span.col));

    let mut entries = Vec::new();
    for s in prog.specs.iter().filter(|s| reachable.contains(&s.id)) {
        let f = &prog.module.funcs[s.key.func];
        let recursive = reaches(s.id, s.id);
        let mut effects: Vec<&'static str> = eff[s.id].iter().copied().collect();
        if recursive && !effects.contains(&"may_raise") {
            effects.push("may_raise"); // RecursionError
        }
        if effects.is_empty() {
            effects.push("pure");
        }
        let ret = match s.ret {
            Slot::Known(t) => t.py_name().to_string(),
            Slot::Boxed(..) => "object".into(),
            _ => "?".into(),
        };
        let params: Vec<String> = f.params.iter().zip(&s.key.args).map(|(p, t)| format!("{p}: {t}")).collect();
        entries.push(PlanEntry {
            spec_id: s.id,
            function: f.name.clone(),
            signature: if f.is_module { pyfast_infer::spec_label(prog, s) } else { format!("{}({}) -> {ret}", f.name, params.join(", ")) },
            rust_name: s.mangled.clone(),
            tier: if per_spec_bridge[s.id] { Tier::Bridge } else { Tier::Native },
            recursive,
            effects,
            span: f.span,
        });
    }
    let reached: HashSet<FuncId> = prog.specs.iter().filter(|s| reachable.contains(&s.id)).map(|s| s.key.func).collect();
    let unreachable = prog.module.funcs.iter().filter(|f| !reached.contains(&f.id) && !f.is_module).map(|f| f.name.clone()).collect();

    let mut diags = Vec::new();
    if prog.mode == Mode::NativeOnly {
        for b in &boundaries {
            diags.push(
                Diagnostic::error(b.code, b.span, format!("{} in {}", b.what, b.function))
                    .note(format!("reason: {}", b.reason))
                    .note("native-only: compilation error (use --mode hybrid to bridge through CPython)"),
            );
        }
    }
    // Native values can hold Python objects (`list[object]`) without any
    // boundary operation: the runtime still needs the embedded interpreter.
    let mut holds_obj = prog.globals.values().any(|s| matches!(s, Slot::Boxed(..)) || s.known().is_some_and(|t| t.mentions_obj()));
    for s in prog.specs.iter().filter(|s| reachable.contains(&s.id)) {
        if holds_obj {
            break;
        }
        holds_obj |= s.locals.values().any(|x| x.known().is_some_and(|t| t.mentions_obj()));
        holds_obj |= s.key.args.iter().any(|t| t.mentions_obj());
        let f = &prog.module.funcs[s.key.func];
        // Callee positions (`s.split`) and slice nodes are not values at run time.
        walk_live(prog, s, &f.body, &mut |e| {
            if !holds_obj && !matches!(e.kind, ExprKind::Slice { .. } | ExprKind::GetAttr { .. }) && prog.type_of(s, e).is_some_and(|t| t.mentions_obj()) {
                holds_obj = true;
            }
        });
    }
    let uses_python = holds_obj || per_spec_bridge.iter().any(|b| *b);
    (Plan { entries, unreachable, boundaries, uses_python, externs_used, reachable }, diags)
}

fn short(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Local(n) => n.clone(),
        ExprKind::Slot(_) => "<import>".into(),
        ExprKind::GetAttr { obj, name } => format!("{}.{name}", short(obj)),
        ExprKind::PyBuiltin(n) => n.clone(),
        ExprKind::Call { .. } | ExprKind::CallObj { .. } => "<call>".into(),
        ExprKind::Str(_) => "<str>".into(),
        _ => "<expr>".into(),
    }
}

fn find_boundaries(prog: &Program, s: &Spec, label: &str, out: &mut Vec<Boundary>) {
    let f = &prog.module.funcs[s.key.func];
    let slot_name = |e: &Expr| -> Option<String> {
        if let ExprKind::Slot(i) = e.kind {
            Some(prog.module.slots[i].binding.clone())
        } else {
            None
        }
    };
    let name_of = |e: &Expr| -> String {
        match &e.kind {
            ExprKind::GetAttr { obj, name } => format!("{}.{name}", slot_name(obj).unwrap_or_else(|| short(obj))),
            _ => slot_name(e).unwrap_or_else(|| short(e)),
        }
    };
    let mut push = |code: &'static str, span: Span, what: String, reason: &str| {
        out.push(Boundary { code, span, function: label.to_string(), what, reason: reason.to_string() })
    };
    for (name, slot) in &s.locals {
        if let Slot::Boxed(a, b) = slot {
            push("P215", f.span, format!("variable `{name}` holds {a} and {b}"), "boxed as a Python object (no common native type)");
        }
    }
    if let Slot::Boxed(a, b) = s.ret {
        push("P215", f.span, format!("return value is {a} or {b}"), "boxed as a Python object (no common native type)");
    }
    let ty0 = |e: &Expr| prog.type_of(s, e);
    walk_stmts(&f.body, &mut |st| match &st.kind {
        StmtKind::Import(ImportAction::Py(slot)) => {
            let sl = &prog.module.slots[*slot];
            let m = match &sl.kind {
                SlotKind::Import { module, .. } => module.clone(),
                SlotKind::From { module, attr } => format!("{module}.{attr}"),
            };
            push("P200", st.span, format!("runtime import of `{m}`"), "no .py/.rs/.c of that name next to the source: CPython import");
        }
        StmtKind::ForIter { iter, .. } if ty0(iter) == Some(Ty::Obj) => push("P214", st.span, "for-loop over a Python iterable".into(), "CPython iterator protocol"),
        StmtKind::SetItem { obj, .. } if ty0(obj) == Some(Ty::Obj) => push("P212", st.span, "item assignment".into(), "CPython __setitem__"),
        StmtKind::Del(t) => match &t.kind {
            ExprKind::Subscript { obj, .. } if ty0(obj) == Some(Ty::Obj) => push("P212", st.span, "item deletion".into(), "CPython __delitem__"),
            ExprKind::GetAttr { name, .. } => push("P210", st.span, format!("attribute deletion `.{name}`"), "CPython __delattr__"),
            _ => {}
        },
        StmtKind::SetAttr { name, obj, .. } if ty0(obj) == Some(Ty::Obj) => push("P210", st.span, format!("attribute assignment `.{name}`"), "CPython __setattr__"),
        StmtKind::With { .. } => push("P216", st.span, "`with` statement".into(), "CPython context manager protocol (__enter__/__exit__)"),
        StmtKind::Try { handlers, .. } => {
            for h in handlers {
                if h.types.iter().any(|t| matches!(t, ExcType::Obj(_))) {
                    push("P216", h.span, "`except` with a Python exception class".into(), "matched with isinstance through CPython");
                }
            }
        }
        _ => {}
    });
    let ty = |e: &Expr| prog.type_of(s, e);
    walk_live(prog, s, &f.body, &mut |e| match &e.kind {
        // Untyped expressions are in code this specialization never runs.
        _ if ty(e).is_none() => {}
        ExprKind::CallObj { func, args, kwargs } if !matches!(prog.classify_call(s, func, args, kwargs), CallClass::Bridge | CallClass::Pending) => {}
        ExprKind::CallObj { func, .. } => {
            let reason = match &func.kind {
                ExprKind::GetAttr { .. } => "CPython attribute lookup + call",
                ExprKind::PyBuiltin(_) => "builtin without a native implementation: CPython builtin",
                ExprKind::Slot(_) => "call of an imported Python object",
                _ => "call of a Python object",
            };
            push("P210", e.span, format!("{}(...)", name_of(func)), reason);
        }
        ExprKind::GetAttr { name, obj } if ty(obj) == Some(Ty::Obj) || ty(obj).is_some_and(|t| !pyfast_infer::spec::class_like(t) && !matches!(t.as_comp(), Some(Comp::Type(..)))) => {
            push("P210", e.span, format!("attribute `.{name}`"), "dynamic receiver: CPython attribute lookup")
        }
        ExprKind::List(_) if ty(e) == Some(Ty::Obj) => push("P212", e.span, "list literal".into(), "kept as a Python list"),
        ExprKind::Dict(_) if ty(e) == Some(Ty::Obj) => push("P212", e.span, "dict literal".into(), "kept as a Python dict (keys are not natively hashable)"),
        ExprKind::Set(_) if ty(e) == Some(Ty::Obj) => push("P212", e.span, "set literal".into(), "kept as a Python set (elements are not natively hashable)"),
        ExprKind::Subscript { obj, .. } if ty(obj) == Some(Ty::Obj) => push("P212", e.span, "subscript".into(), "CPython __getitem__"),
        ExprKind::PyBuiltin(n) if !matches!(n.as_str(), "int" | "float" | "str" | "bool" | "list" | "dict" | "set" | "tuple" | "object") => {
            push("P211", e.span, format!("builtin `{n}` used as a value"), "CPython builtin object")
        }
        ExprKind::BinOp { op, left, right } if ty(left) == Some(Ty::Obj) || ty(right) == Some(Ty::Obj) => {
            push("P213", e.span, format!("operator `{}` on a Python object", op.symbol()), "CPython number protocol")
        }
        ExprKind::Compare { first, rest } if std::iter::once(&**first).chain(rest.iter().map(|r| &r.1)).any(|x| ty(x) == Some(Ty::Obj)) => {
            push("P213", e.span, "comparison involving a Python object".into(), "CPython rich comparison")
        }
        _ => {}
    });
}

fn local_effects(prog: &Program, s: &Spec, e: &Expr, out: &mut BTreeSet<&'static str>) {
    match &e.kind {
        ExprKind::Call { callee: Callee::Builtin(b), args, .. } => match b {
            Builtin::Print | Builtin::PrintKw => {
                out.insert("io");
            }
            Builtin::Int | Builtin::Float | Builtin::Abs => {
                if args.iter().any(|a| !matches!(prog.type_of(s, a), Some(Ty::Bool))) {
                    out.insert("may_raise");
                }
            }
            _ => {}
        },
        ExprKind::ExternCall { .. } => {
            out.insert("ffi");
        }
        ExprKind::BinOp { op, left, right } => {
            let (l, r) = (prog.type_of(s, left), prog.type_of(s, right));
            let float_only = l == Some(Ty::Float) || r == Some(Ty::Float);
            let raises = match op {
                BinOp::Div | BinOp::FloorDiv | BinOp::Mod | BinOp::Pow | BinOp::LShift | BinOp::RShift => true,
                BinOp::Add | BinOp::Sub | BinOp::Mul => !float_only && l != Some(Ty::Str),
                _ => false,
            };
            if raises {
                out.insert("may_raise");
            }
        }
        ExprKind::Unary { op: UnaryOp::Neg, operand } if prog.type_of(s, operand) == Some(Ty::Int) => {
            out.insert("may_raise");
        }
        ExprKind::Local(n) if s.maybe_unbound.contains(n) => {
            out.insert("may_raise");
        }
        _ => {}
    }
}

/// Visit every statement (pre-order, including nested blocks).
pub fn walk_stmts(stmts: &[Stmt], f: &mut impl FnMut(&Stmt)) {
    for s in stmts {
        f(s);
        match &s.kind {
            StmtKind::If { body, orelse, .. } => {
                walk_stmts(body, f);
                walk_stmts(orelse, f);
            }
            StmtKind::While { body, .. } | StmtKind::ForRange { body, .. } | StmtKind::ForIter { body, .. } | StmtKind::With { body, .. } => {
                walk_stmts(body, f)
            }
            StmtKind::Try { body, handlers, orelse, finalbody } => {
                walk_stmts(body, f);
                for h in handlers {
                    walk_stmts(&h.body, f);
                }
                walk_stmts(orelse, f);
                walk_stmts(finalbody, f);
            }
            _ => {}
        }
    }
}

fn stmt_exprs(s: &Stmt) -> Vec<&Expr> {
    match &s.kind {
        StmtKind::Assign { value, .. } | StmtKind::AugAssign { value, .. } | StmtKind::Expr(value) => vec![value],
        StmtKind::Return(Some(v)) => vec![v],
        StmtKind::If { test, .. } | StmtKind::While { test, .. } => vec![test],
        StmtKind::ForRange { start, stop, step, .. } => [start.as_ref(), Some(stop), step.as_ref()].into_iter().flatten().collect(),
        StmtKind::ForIter { iter, .. } => vec![iter],
        StmtKind::SetItem { obj, index, value } => vec![obj, index, value],
        StmtKind::SetAttr { obj, value, .. } => vec![obj, value],
        StmtKind::Raise(Some(e)) => vec![e],
        StmtKind::Del(e) => vec![e],
        StmtKind::AssignGlobal { value, .. } => vec![value],
        StmtKind::With { ctx, .. } => vec![ctx],
        StmtKind::Try { handlers, .. } => handlers
            .iter()
            .flat_map(|h| h.types.iter())
            .filter_map(|t| if let ExcType::Obj(e) = t { Some(e) } else { None })
            .collect(),
        _ => vec![],
    }
}

pub fn walk_block(stmts: &[Stmt], f: &mut impl FnMut(&Expr)) {
    walk_stmts(stmts, &mut |s| {
        for e in stmt_exprs(s) {
            walk_expr(e, f, false);
        }
    });
}

/// Statements and expressions the specialization can execute: branches
/// decided by its types (`x is None` with `x: None`, `if False`) are skipped.
fn walk_live(prog: &Program, s: &Spec, stmts: &[Stmt], f: &mut impl FnMut(&Expr)) {
    for st in stmts {
        match &st.kind {
            StmtKind::If { test, body, orelse } => match prog.static_truth(s, test) {
                Some(true) => walk_live(prog, s, body, f),
                Some(false) => walk_live(prog, s, orelse, f),
                None => {
                    walk_expr_live(prog, s, test, f);
                    walk_live(prog, s, body, f);
                    walk_live(prog, s, orelse, f);
                }
            },
            StmtKind::While { test, body } => {
                walk_expr_live(prog, s, test, f);
                if prog.static_truth(s, test) != Some(false) {
                    walk_live(prog, s, body, f);
                }
            }
            _ => {
                for e in stmt_exprs(st) {
                    walk_expr_live(prog, s, e, f);
                }
                match &st.kind {
                    StmtKind::ForRange { body, .. } | StmtKind::ForIter { body, .. } | StmtKind::With { body, .. } => walk_live(prog, s, body, f),
                    StmtKind::Try { body, handlers, orelse, finalbody } => {
                        walk_live(prog, s, body, f);
                        for h in handlers {
                            walk_live(prog, s, &h.body, f);
                        }
                        walk_live(prog, s, orelse, f);
                        walk_live(prog, s, finalbody, f);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn walk_expr_live(prog: &Program, s: &Spec, e: &Expr, f: &mut impl FnMut(&Expr)) {
    match &e.kind {
        ExprKind::IfExp { test, body, orelse } => match prog.static_truth(s, test) {
            Some(true) => walk_expr_live(prog, s, body, f),
            Some(false) => walk_expr_live(prog, s, orelse, f),
            None => {
                f(e);
                for c in [test, body, orelse] {
                    walk_expr_live(prog, s, c, f);
                }
            }
        },
        ExprKind::BoolOp { op, values } => {
            f(e);
            let n = prog.short_circuit(s, *op, values).unwrap_or(values.len());
            values[..n].iter().for_each(|v| walk_expr_live(prog, s, v, f));
        }
        // `type(x).__name__` of a native value is a plain string.
        ExprKind::GetAttr { obj, name } if name == "__name__" && prog.type_of(s, e) == Some(Ty::Str) && pyfast_infer::spec::type_call_arg(obj).is_some() => {
            walk_expr_live(prog, s, pyfast_infer::spec::type_call_arg(obj).unwrap(), f);
        }
        _ => {
            f(e);
            for c in children(e, true) {
                walk_expr_live(prog, s, c, f);
            }
        }
    }
}

/// Direct sub-expressions (without the callee lookup of a `CallObj` when
/// `skip_callee`, but with the receiver of `recv.method(...)`).
fn children(e: &Expr, skip_callee: bool) -> Vec<&Expr> {
    match &e.kind {
        ExprKind::BinOp { left, right, .. } => vec![left, right],
        ExprKind::Unary { operand, .. } => vec![operand],
        ExprKind::Compare { first, rest } => std::iter::once(&**first).chain(rest.iter().map(|r| &r.1)).collect(),
        ExprKind::BoolOp { values, .. } => values.iter().collect(),
        ExprKind::IfExp { test, body, orelse } => vec![test, body, orelse],
        ExprKind::Call { args, .. } | ExprKind::ExternCall { args, .. } | ExprKind::List(args) | ExprKind::Tuple(args) | ExprKind::NewExc { args, .. } | ExprKind::Set(args) => args.iter().collect(),
        ExprKind::GetAttr { obj, .. } | ExprKind::Annotated { value: obj, .. } | ExprKind::Unpack { value: obj, .. } | ExprKind::Starred(obj) => vec![obj],
        ExprKind::CallObj { func, args, kwargs } => {
            let mut v: Vec<&Expr> = Vec::new();
            if skip_callee {
                if let ExprKind::GetAttr { obj, .. } = &func.kind {
                    if !matches!(obj.kind, ExprKind::Slot(_)) {
                        v.push(obj);
                    }
                }
            } else {
                v.push(func);
            }
            v.extend(args.iter());
            v.extend(kwargs.iter().map(|(_, x)| x));
            v
        }
        ExprKind::New { args, kwargs, .. } => args.iter().chain(kwargs.iter().map(|(_, x)| x)).collect(),
        ExprKind::Dict(pairs) => pairs.iter().flat_map(|(k, v)| [k, v]).collect(),
        ExprKind::Subscript { obj, index } => vec![obj, index],
        ExprKind::Slice { lo, hi, step } => [lo, hi, step].into_iter().flatten().map(|b| &**b).collect(),
        ExprKind::Comp { elt, val, gens, .. } => {
            let mut v: Vec<&Expr> = Vec::new();
            for g in gens {
                v.push(&g.iter);
                for st in &g.unpack {
                    v.extend(stmt_exprs(st));
                    if let StmtKind::If { body, .. } = &st.kind {
                        for b in body {
                            v.extend(stmt_exprs(b));
                        }
                    }
                }
                v.extend(g.ifs.iter());
            }
            v.push(elt);
            if let Some(x) = val {
                v.push(x);
            }
            v
        }
        _ => vec![],
    }
}

pub fn walk_expr(e: &Expr, f: &mut impl FnMut(&Expr), skip_callee: bool) {
    f(e);
    match &e.kind {
        ExprKind::BinOp { left, right, .. } => {
            walk_expr(left, f, skip_callee);
            walk_expr(right, f, skip_callee);
        }
        ExprKind::Unary { operand, .. } => walk_expr(operand, f, skip_callee),
        ExprKind::Compare { first, rest } => {
            walk_expr(first, f, skip_callee);
            for (_, c) in rest {
                walk_expr(c, f, skip_callee);
            }
        }
        ExprKind::BoolOp { values, .. } => values.iter().for_each(|v| walk_expr(v, f, skip_callee)),
        ExprKind::IfExp { test, body, orelse } => {
            walk_expr(test, f, skip_callee);
            walk_expr(body, f, skip_callee);
            walk_expr(orelse, f, skip_callee);
        }
        ExprKind::Slice { lo, hi, step } => [lo, hi, step].into_iter().flatten().for_each(|a| walk_expr(a, f, skip_callee)),
        ExprKind::Unpack { value, .. } | ExprKind::Starred(value) => walk_expr(value, f, skip_callee),
        ExprKind::New { args, kwargs, .. } => {
            args.iter().for_each(|a| walk_expr(a, f, skip_callee));
            kwargs.iter().for_each(|(_, v)| walk_expr(v, f, skip_callee));
        }
        ExprKind::Comp { elt, val, gens, .. } => {
            for g in gens {
                walk_expr(&g.iter, f, skip_callee);
                for st in &g.unpack {
                    for x in stmt_exprs(st) {
                        walk_expr(x, f, skip_callee);
                    }
                    if let StmtKind::If { body, .. } = &st.kind {
                        for b in body {
                            for x in stmt_exprs(b) {
                                walk_expr(x, f, skip_callee);
                            }
                        }
                    }
                }
                for c in &g.ifs {
                    walk_expr(c, f, skip_callee);
                }
            }
            walk_expr(elt, f, skip_callee);
            if let Some(v) = val {
                walk_expr(v, f, skip_callee);
            }
        }
        ExprKind::Call { args, .. } | ExprKind::ExternCall { args, .. } | ExprKind::List(args) | ExprKind::Tuple(args) | ExprKind::NewExc { args, .. } | ExprKind::Set(args) => {
            args.iter().for_each(|a| walk_expr(a, f, skip_callee))
        }
        ExprKind::GetAttr { obj, .. } | ExprKind::Annotated { value: obj, .. } => walk_expr(obj, f, skip_callee),
        ExprKind::CallObj { func, args, kwargs } => {
            if skip_callee {
                // Visit the receiver of `recv.method(...)` but not the lookup itself.
                if let ExprKind::GetAttr { obj, .. } = &func.kind {
                    if !matches!(obj.kind, ExprKind::Slot(_)) {
                        walk_expr(obj, f, skip_callee);
                    }
                }
            } else {
                walk_expr(func, f, skip_callee);
            }
            args.iter().for_each(|a| walk_expr(a, f, skip_callee));
            kwargs.iter().for_each(|(_, v)| walk_expr(v, f, skip_callee));
        }
        ExprKind::Dict(pairs) => pairs.iter().for_each(|(k, v)| {
            walk_expr(k, f, skip_callee);
            walk_expr(v, f, skip_callee);
        }),
        ExprKind::Subscript { obj, index } => {
            walk_expr(obj, f, skip_callee);
            walk_expr(index, f, skip_callee);
        }
        _ => {}
    }
}
