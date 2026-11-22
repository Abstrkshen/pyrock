//! Eligibility planning (design section 4): decides, per specialization,
//! whether code is fully native or crosses into CPython, records every
//! boundary with its reason, and summarises effects for the explain report.
//!
//! In `native-only` mode each boundary becomes a compile error.

use pyfast_infer::{Program, Slot, Spec};
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
}

impl Plan {
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
        if let ExprKind::Call { callee: Callee::Func(fid), args } = &e.kind {
            let tys: Option<Vec<Ty>> = args.iter().map(|a| prog.type_of(s, a)).collect();
            if let Some(args) = tys {
                if let Some(c) = prog.spec(&pyfast_infer::SpecKey { func: *fid, args }) {
                    out.insert(c.id);
                }
            }
        }
    });
    walk_stmts(&f.body, &mut |st| {
        if let StmtKind::Import(ImportAction::Unit(fid)) = &st.kind {
            if let Some(c) = prog.spec(&pyfast_infer::SpecKey { func: *fid, args: vec![] }) {
                out.insert(c.id);
            }
        }
    });
    out
}

/// Call edges from each specialization to the specializations it calls.
pub fn call_graph(prog: &Program) -> Vec<BTreeSet<usize>> {
    prog.specs.iter().map(|s| spec_calls(prog, s)).collect()
}

pub fn plan(prog: &Program) -> (Plan, Vec<Diagnostic>) {
    let g = call_graph(prog);
    let n = prog.specs.len();
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
    for s in &prog.specs {
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
    let reached: HashSet<FuncId> = prog.specs.iter().map(|s| s.key.func).collect();
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
    let uses_python = per_spec_bridge.iter().any(|b| *b);
    (Plan { entries, unreachable, boundaries, uses_python, externs_used }, diags)
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
    walk_stmts(&f.body, &mut |st| match &st.kind {
        StmtKind::Import(ImportAction::Py(slot)) => {
            let sl = &prog.module.slots[*slot];
            let m = match &sl.kind {
                SlotKind::Import { module, .. } => module.clone(),
                SlotKind::From { module, attr } => format!("{module}.{attr}"),
            };
            push("P200", st.span, format!("runtime import of `{m}`"), "no .py/.rs/.c of that name next to the source: CPython import");
        }
        StmtKind::ForIter { .. } => push("P214", st.span, "for-loop over a Python iterable".into(), "CPython iterator protocol"),
        StmtKind::SetItem { .. } => push("P212", st.span, "item assignment".into(), "CPython __setitem__"),
        StmtKind::SetAttr { name, .. } => push("P210", st.span, format!("attribute assignment `.{name}`"), "CPython __setattr__"),
        _ => {}
    });
    let ty = |e: &Expr| prog.type_of(s, e);
    walk_block_skip_callees(&f.body, &mut |e| match &e.kind {
        ExprKind::CallObj { func, .. } => {
            let reason = match &func.kind {
                ExprKind::GetAttr { .. } => "CPython attribute lookup + call",
                ExprKind::PyBuiltin(_) => "builtin without a native implementation: CPython builtin",
                ExprKind::Slot(_) => "call of an imported Python object",
                _ => "call of a Python object",
            };
            push("P210", e.span, format!("{}(...)", name_of(func)), reason);
        }
        ExprKind::GetAttr { name, .. } => push("P210", e.span, format!("attribute `.{name}`"), "dynamic receiver: CPython attribute lookup"),
        ExprKind::List(_) => push("P212", e.span, "list literal".into(), "mutable collection kept as a Python list (preserves aliasing)"),
        ExprKind::Dict(_) => push("P212", e.span, "dict literal".into(), "mutable collection kept as a Python dict (preserves aliasing)"),
        ExprKind::Tuple(_) => push("P212", e.span, "tuple literal".into(), "tuples are Python objects until native tuples land"),
        ExprKind::Subscript { .. } => push("P212", e.span, "subscript".into(), "CPython __getitem__"),
        ExprKind::PyBuiltin(n) => push("P211", e.span, format!("builtin `{n}` used as a value"), "CPython builtin object"),
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
        ExprKind::Call { callee: Callee::Builtin(b), args } => match b {
            Builtin::Print => {
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
            StmtKind::While { body, .. } | StmtKind::ForRange { body, .. } | StmtKind::ForIter { body, .. } => walk_stmts(body, f),
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

/// Like `walk_block`, but the callee of a `CallObj` is not visited itself
/// (the call is reported as one boundary).
fn walk_block_skip_callees(stmts: &[Stmt], f: &mut impl FnMut(&Expr)) {
    walk_stmts(stmts, &mut |s| {
        for e in stmt_exprs(s) {
            walk_expr(e, f, true);
        }
    });
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
        ExprKind::Call { args, .. } | ExprKind::ExternCall { args, .. } | ExprKind::List(args) | ExprKind::Tuple(args) => {
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
