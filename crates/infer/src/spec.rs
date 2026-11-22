//! Reachability-driven specialization (design section 5).
//!
//! Starting at the module body, every call with concrete argument types is
//! keyed by (function, argument types) and analysed separately. Analysis is
//! a monotone fixed point over the lattice Unknown < Known(T) < Boxed/Conflict,
//! so recursion is handled by starting each return type at Unknown (the
//! "placeholder") and iterating until nothing changes.
//!
//! In hybrid mode a value that takes several types (or meets a CPython
//! object) is *boxed*: it becomes a Python object instead of an error.

use crate::{definite, ops};
use pyfast_ir::*;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MAX_SPECS_PER_FUNCTION: usize = 16;
pub const MAX_ROUNDS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpecKey {
    pub func: FuncId,
    pub args: Vec<Ty>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Unknown,
    Known(Ty),
    /// Held several types (first two shown); represented as a Python object.
    Boxed(Ty, Ty),
    /// Held several types in native-only mode: a compile error.
    Conflict(Ty, Ty),
}

/// `None` joined with `T` is `T | None` (a native `Option<T>`), in every mode.
pub fn opt_join(a: Ty, b: Ty) -> Option<Ty> {
    let base = |t: Ty| t.as_opt().unwrap_or(t);
    match (a, b) {
        _ if a == Ty::Obj || b == Ty::Obj => None,
        (Ty::None, x) | (x, Ty::None) => Some(Ty::opt(x)),
        _ if base(a) == base(b) && (a.as_opt().is_some() || b.as_opt().is_some()) => Some(Ty::opt(base(a))),
        _ => None,
    }
}

impl Slot {
    fn join(self, t: Ty, boxing: bool) -> Slot {
        match self {
            Slot::Unknown => Slot::Known(t),
            Slot::Known(a) if a == t => self,
            Slot::Known(a) if opt_join(a, t).is_some() => Slot::Known(opt_join(a, t).unwrap()),
            Slot::Known(a) if boxing || a == Ty::Obj || t == Ty::Obj => Slot::Boxed(a, t),
            Slot::Known(a) => Slot::Conflict(a, t),
            c => c,
        }
    }
    pub fn known(self) -> Option<Ty> {
        match self {
            Slot::Known(t) => Some(t),
            Slot::Boxed(..) => Some(Ty::Obj),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub id: usize,
    pub key: SpecKey,
    /// Rust-level name, e.g. `square__float`.
    pub mangled: String,
    pub ret: Slot,
    /// Whether control can fall off the end (implicit `return None`).
    pub falls_through: bool,
    /// False until the first analysis pass; callers see Unknown until then.
    pub analyzed: bool,
    pub locals: BTreeMap<String, Slot>,
    pub maybe_unbound: HashSet<String>,
}

pub struct Program<'m> {
    pub module: &'m Module,
    pub mode: Mode,
    pub specs: Vec<Spec>,
    pub index: HashMap<SpecKey, usize>,
    pub diagnostics: Vec<Diagnostic>,
}

impl<'m> Program<'m> {
    pub fn boxing(&self) -> bool {
        self.mode == Mode::Hybrid
    }

    /// The type a call to `s` produces, as currently known.
    pub fn call_result(&self, s: &Spec) -> Option<Ty> {
        if !s.analyzed {
            return None;
        }
        match (s.ret, s.falls_through) {
            (Slot::Known(Ty::None), _) | (Slot::Unknown, true) => Some(Ty::None),
            (Slot::Known(t), false) => Some(t),
            // Falling off the end returns None: `T | None`.
            (Slot::Known(t), true) => Some(opt_join(t, Ty::None).unwrap_or(Ty::Obj)),
            (Slot::Boxed(..), _) => Some(Ty::Obj),
            _ => None,
        }
    }

    pub fn spec(&self, key: &SpecKey) -> Option<&Spec> {
        self.index.get(key).map(|&i| &self.specs[i])
    }

    pub fn main_spec(&self) -> &Spec {
        &self.specs[0]
    }

    /// Truth value of a test that is decided by the specialization's types:
    /// `x is None` / `x is not None` on a local whose type is exactly `None`
    /// or cannot be `None`, combined with `not`, `and`, `or`. Only
    /// side-effect-free tests fold, so skipping their evaluation is exact.
    pub fn static_truth(&self, spec: &Spec, e: &Expr) -> Option<bool> {
        static_truth(&spec.locals, e)
    }

    pub fn short_circuit(&self, spec: &Spec, op: BoolOpKind, values: &[Expr]) -> Option<usize> {
        short_circuit(&spec.locals, op, values)
    }
}

pub fn static_truth(locals: &BTreeMap<String, Slot>, e: &Expr) -> Option<bool> {
    {
        let none_state = |x: &Expr| -> Option<bool> {
            match &x.kind {
                ExprKind::NoneLit => Some(true),
                ExprKind::Local(n) => match locals.get(n)?.known()? {
                    Ty::None => Some(true),
                    Ty::Obj => None,
                    t if t.as_opt().is_some() => None,
                    _ => Some(false),
                },
                _ => None,
            }
        };
        match &e.kind {
            ExprKind::Bool(b) => Some(*b),
            ExprKind::Compare { first, rest } if rest.len() == 1 && matches!(rest[0].0, CmpOp::Is | CmpOp::IsNot) => {
                let (a, b) = (none_state(first)?, none_state(&rest[0].1)?);
                // Only `x is None` forms: at least one side must be the None literal.
                if !matches!(first.kind, ExprKind::NoneLit) && !matches!(rest[0].1.kind, ExprKind::NoneLit) {
                    return None;
                }
                let is = a && b;
                Some(if rest[0].0 == CmpOp::Is { is } else { !is })
            }
            ExprKind::Unary { op: UnaryOp::Not, operand } => static_truth(locals, operand).map(|v| !v),
            ExprKind::BoolOp { op, values } => {
                // Decided only if every operand is decided (no side effects to skip).
                let vs: Option<Vec<bool>> = values.iter().map(|v| static_truth(locals, v)).collect();
                let vs = vs?;
                Some(if *op == BoolOpKind::And { vs.iter().all(|v| *v) } else { vs.iter().any(|v| *v) })
            }
            _ => None,
        }
    }

}

/// For `a and b and c`: the number of leading operands evaluated when an
/// operand's static truth short-circuits (`None` when not decided).
pub fn short_circuit(locals: &BTreeMap<String, Slot>, op: BoolOpKind, values: &[Expr]) -> Option<usize> {
    {
        for (i, v) in values.iter().enumerate().take(values.len() - 1) {
            match (static_truth(locals, v), op) {
                (Some(false), BoolOpKind::And) | (Some(true), BoolOpKind::Or) => return Some(i + 1),
                _ => {}
            }
        }
        None
    }
}

impl Program<'_> {
    /// Type of `e` under the locals of `spec`. Returns `None` when unknown or ill-typed.
    pub fn type_of(&self, spec: &Spec, e: &Expr) -> Option<Ty> {
        let mut ctx = Ctx { prog: self, locals: &spec.locals, requests: None, diags: None };
        ctx.expr(e)
    }
}

pub fn specialize(module: &Module, mode: Mode) -> Program<'_> {
    let mut prog = Program { module, mode, specs: vec![], index: HashMap::new(), diagnostics: vec![] };
    let main_key = SpecKey { func: module.main, args: vec![] };
    add_spec(&mut prog, main_key);

    let mut rounds = 0;
    loop {
        rounds += 1;
        let mut changed = false;
        let mut i = 0;
        while i < prog.specs.len() {
            let mut requests = Vec::new();
            let (locals, ret, falls) = analyze(&prog, i, &mut requests, None);
            let s = &mut prog.specs[i];
            if !s.analyzed || s.locals != locals || s.ret != ret || s.falls_through != falls {
                s.analyzed = true;
                s.locals = locals;
                s.ret = ret;
                s.falls_through = falls;
                changed = true;
            }
            for (key, span) in requests {
                if !prog.index.contains_key(&key) {
                    let count = prog.specs.iter().filter(|s| s.key.func == key.func).count();
                    if count >= MAX_SPECS_PER_FUNCTION {
                        let name = &module.funcs[key.func].name;
                        prog.diagnostics.push(Diagnostic::error(
                            "P310",
                            span,
                            format!("specialization limit ({MAX_SPECS_PER_FUNCTION}) reached for `{name}`"),
                        ));
                        continue;
                    }
                    add_spec(&mut prog, key);
                    changed = true;
                }
            }
            i += 1;
        }
        if !changed {
            break;
        }
        if rounds >= MAX_ROUNDS {
            prog.diagnostics.push(Diagnostic::error("P311", Span::default(), "type inference did not converge"));
            break;
        }
    }

    // Final reporting pass over the converged state.
    let mut diags = Vec::new();
    for i in 0..prog.specs.len() {
        let mut sink = Vec::new();
        let before = diags.len();
        analyze(&prog, i, &mut sink, Some(&mut diags));
        let body_errors = diags.len() > before;
        let spec = &prog.specs[i];
        let f = &module.funcs[spec.key.func];
        for (name, slot) in &spec.locals {
            if let Slot::Conflict(a, b) = slot {
                diags.push(
                    Diagnostic::error("P304", f.span, format!("`{name}` in {} is bound to both {a} and {b}", spec_label(&prog, spec)))
                        .note("a closed union or boxed value is required (milestone M4); give each value its own name for now"),
                );
            }
        }
        if !f.is_module {
            let boxing = prog.boxing();
            let (ret, ok) = match (spec.ret, spec.falls_through) {
                (Slot::Known(t), false) => (Some(Slot::Known(t)), true),
                (Slot::Known(Ty::None), true) => (Some(Slot::Known(Ty::None)), true),
                (Slot::Unknown, true) => (Some(Slot::Known(Ty::None)), true),
                (Slot::Unknown, false) => (None, false),
                (Slot::Boxed(a, b), _) => (Some(Slot::Boxed(a, b)), true),
                (Slot::Known(t), true) if opt_join(t, Ty::None).is_some() => (Some(Slot::Known(opt_join(t, Ty::None).unwrap())), true),
                (Slot::Known(t), true) if boxing => (Some(Slot::Boxed(t, Ty::None)), true),
                (Slot::Known(t), true) => {
                    diags.push(
                        Diagnostic::error("P305", f.span, format!("{} returns {t} on some paths and None on others", spec_label(&prog, spec)))
                            .note("Optional[T] is milestone M4"),
                    );
                    (Some(Slot::Known(t)), false)
                }
                (Slot::Conflict(a, b), _) => {
                    diags.push(
                        Diagnostic::error("P305", f.span, format!("{} returns both {a} and {b}", spec_label(&prog, spec)))
                            .note("a closed union (e.g. IntOrStr) is milestone M4"),
                    );
                    (Some(Slot::Known(a)), false)
                }
            };
            // Skip when an error in the body already explains the unknown type.
            if !ok && ret.is_none() && !body_errors {
                diags.push(
                    Diagnostic::error("P306", f.span, format!("could not infer a return type for {}", spec_label(&prog, spec)))
                        .note("recursion without a base case never produces a value"),
                );
            }
            if let Some(r) = ret {
                prog.specs[i].ret = r;
            }
        }
    }
    dedup(&mut diags);
    prog.diagnostics.extend(diags);
    prog
}

fn dedup(d: &mut Vec<Diagnostic>) {
    let mut seen = HashSet::new();
    d.retain(|x| seen.insert((x.code, x.span, x.message.clone())));
}

pub fn spec_label(prog: &Program, s: &Spec) -> String {
    let f = &prog.module.funcs[s.key.func];
    if f.is_module {
        return if f.unit == 0 { "<module>".into() } else { format!("<module {}>", prog.module.units[f.unit].name) };
    }
    format!("{}({})", f.name, s.key.args.iter().map(|t| t.py_name()).collect::<Vec<_>>().join(", "))
}

fn add_spec(prog: &mut Program, key: SpecKey) {
    let f = &prog.module.funcs[key.func];
    let mangled = if f.is_module {
        if f.unit == 0 { "module_body".to_string() } else { format!("init_{}", prog.module.units[f.unit].name) }
    } else if key.args.is_empty() {
        format!("{}__", f.rust_base())
    } else {
        format!("{}__{}", f.rust_base(), key.args.iter().map(|t| t.mangle()).collect::<Vec<_>>().join("_"))
    };
    let mut locals = BTreeMap::new();
    for (p, t) in f.params.iter().zip(&key.args) {
        locals.insert(p.clone(), Slot::Known(*t));
    }
    let maybe_unbound = definite::maybe_unbound(f);
    let id = prog.specs.len();
    prog.index.insert(key.clone(), id);
    prog.specs.push(Spec { id, key, mangled, ret: Slot::Unknown, falls_through: true, analyzed: false, locals, maybe_unbound });
}

type Requests = Vec<(SpecKey, Span)>;

/// Analyse one specialization to a local fixed point under the current
/// return types of every other specialization.
fn analyze(
    prog: &Program,
    idx: usize,
    requests: &mut Requests,
    diags: Option<&mut Vec<Diagnostic>>,
) -> (BTreeMap<String, Slot>, Slot, bool) {
    let spec = &prog.specs[idx];
    let f = &prog.module.funcs[spec.key.func];
    let mut locals = spec.locals.clone();
    let mut ret;
    let mut falls;
    let mut guard = 0;
    loop {
        let before = locals.clone();
        ret = Slot::Unknown;
        let mut ctx = Ctx { prog, locals: &locals, requests: Some(requests), diags: None };
        let mut assigns = Vec::new();
        falls = ctx.block(&f.body, &mut assigns, &mut ret);
        for (name, t) in assigns {
            let s = locals.entry(name).or_insert(Slot::Unknown);
            *s = s.join(t, prog.boxing());
        }
        guard += 1;
        if locals == before || guard > 32 {
            break;
        }
    }
    if let Some(d) = diags {
        let mut ret2 = Slot::Unknown;
        let mut sink = Vec::new();
        let mut ctx = Ctx { prog, locals: &locals, requests: None, diags: Some(d) };
        ctx.block(&f.body, &mut sink, &mut ret2);
    }
    (locals, ret, falls)
}

struct Ctx<'a, 'p> {
    prog: &'a Program<'p>,
    locals: &'a BTreeMap<String, Slot>,
    requests: Option<&'a mut Requests>,
    diags: Option<&'a mut Vec<Diagnostic>>,
}

impl Ctx<'_, '_> {
    fn report(&mut self, d: Diagnostic) {
        if let Some(ds) = self.diags.as_deref_mut() {
            ds.push(d);
        }
    }

    /// Returns whether control can fall through the block.
    fn block(&mut self, stmts: &[Stmt], assigns: &mut Vec<(String, Ty)>, ret: &mut Slot) -> bool {
        for s in stmts {
            if !self.stmt(s, assigns, ret) {
                return false;
            }
        }
        true
    }

    fn stmt(&mut self, s: &Stmt, assigns: &mut Vec<(String, Ty)>, ret: &mut Slot) -> bool {
        match &s.kind {
            StmtKind::Assign { target, value } => {
                if let Some(t) = self.expr(value) {
                    assigns.push((target.clone(), t));
                }
            }
            StmtKind::AugAssign { target, op, value } => {
                let cur = Expr { kind: ExprKind::Local(target.clone()), span: s.span };
                let e = Expr {
                    kind: ExprKind::BinOp { op: *op, left: Box::new(cur), right: Box::new(value.clone()) },
                    span: s.span,
                };
                if let Some(t) = self.expr(&e) {
                    assigns.push((target.clone(), t));
                }
            }
            StmtKind::Expr(e) => {
                self.expr(e);
            }
            StmtKind::Return(v) => {
                let t = match v {
                    Some(e) => self.expr(e),
                    None => Some(Ty::None),
                };
                if let Some(t) = t {
                    *ret = ret.join(t, self.prog.boxing());
                }
                return false;
            }
            StmtKind::If { test, body, orelse } => {
                self.expr(test);
                match static_truth(self.locals, test) {
                    Some(true) => return self.block(body, assigns, ret),
                    Some(false) => return self.block(orelse, assigns, ret),
                    None => {}
                }
                let a = self.block(body, assigns, ret);
                let b = self.block(orelse, assigns, ret);
                return a || b;
            }
            StmtKind::While { test, body } => {
                self.expr(test);
                if static_truth(self.locals, test) == Some(false) {
                    return true;
                }
                self.block(body, assigns, ret);
                // `while True:` without break never falls through.
                if matches!(test.kind, ExprKind::Bool(true)) && !contains_break(body) {
                    return false;
                }
            }
            StmtKind::ForRange { var, start, stop, step, body } => {
                for b in [start.as_ref(), Some(stop), step.as_ref()].into_iter().flatten() {
                    if let Some(t) = self.expr(b) {
                        if !matches!(ops::base(t), Ty::Int | Ty::Bool | Ty::Obj) {
                            self.report(Diagnostic::error("P301", b.span, format!("'{t}' object cannot be interpreted as an integer")));
                        }
                    }
                }
                assigns.push((var.clone(), Ty::Int));
                self.block(body, assigns, ret);
            }
            StmtKind::ForIter { var, iter, body } => {
                self.expr(iter);
                assigns.push((var.clone(), Ty::Obj));
                self.block(body, assigns, ret);
            }
            StmtKind::SetItem { obj, index, value } => {
                self.expr(obj);
                self.expr(index);
                self.expr(value);
            }
            StmtKind::SetAttr { obj, value, .. } => {
                self.expr(obj);
                self.expr(value);
            }
            StmtKind::Import(ImportAction::Unit(fid)) => {
                if let Some(req) = self.requests.as_deref_mut() {
                    req.push((SpecKey { func: *fid, args: vec![] }, s.span));
                }
            }
            StmtKind::Import(ImportAction::Py(_)) => {}
            StmtKind::Break | StmtKind::Continue | StmtKind::Pass => {}
            StmtKind::Try { body, handlers, orelse, finalbody } => {
                let mut falls = self.block(body, assigns, ret) && self.block(orelse, assigns, ret);
                for h in handlers {
                    for t in &h.types {
                        if let ExcType::Obj(e) = t {
                            self.expr(e);
                        }
                    }
                    if let Some(n) = &h.name {
                        assigns.push((n.clone(), Ty::exc()));
                    }
                    falls |= self.block(&h.body, assigns, ret);
                }
                // A body that may raise reaches the handlers; one that never
                // raises still falls through when its own end is reachable.
                return self.block(finalbody, assigns, ret) && falls;
            }
            StmtKind::Raise(e) => {
                if let Some(e) = e {
                    if let Some(t) = self.expr(e) {
                        if t != Ty::Obj && t != Ty::exc() {
                            self.report(Diagnostic::error("P301", e.span, "exceptions must derive from BaseException").note("CPython would raise TypeError here"));
                        }
                    }
                }
                return false;
            }
            StmtKind::With { ctx, var, body } => {
                if let Some(t) = self.expr(ctx) {
                    if t != Ty::Obj {
                        self.report(
                            Diagnostic::error("P301", ctx.span, format!("'{t}' object does not support the context manager protocol"))
                                .note("CPython would raise TypeError here"),
                        );
                    }
                }
                if let Some(v) = var {
                    assigns.push((v.clone(), Ty::Obj));
                }
                // `__exit__` may suppress an exception and continue after the block.
                self.block(body, assigns, ret);
            }
        }
        true
    }

    /// Evaluate every sub-expression; the result is an object if all are known.
    fn all_obj<'e>(&mut self, es: impl IntoIterator<Item = &'e Expr>) -> Option<Ty> {
        let mut ok = true;
        for e in es {
            ok &= self.expr(e).is_some();
        }
        ok.then_some(Ty::Obj)
    }

    /// Common type of values flowing into one result (and/or, if-else).
    fn merge(&mut self, tys: &[Ty], e: &Expr, what: &str) -> Option<Ty> {
        let first = tys[0];
        if tys.iter().all(|t| *t == first) {
            return Some(first);
        }
        if let Some(j) = tys[1..].iter().try_fold(first, |acc, t| if acc == *t { Some(acc) } else { opt_join(acc, *t) }) {
            return Some(j);
        }
        if self.prog.boxing() || tys.contains(&Ty::Obj) {
            return Some(Ty::Obj);
        }
        let names: Vec<&str> = tys.iter().map(|t| t.py_name()).collect();
        self.report(
            Diagnostic::error("P302", e.span, format!("{what} yields a value-dependent type ({})", names.join(" or ")))
                .note("native-only mode cannot box; convert explicitly (e.g. bool(...)) or use hybrid mode"),
        );
        None
    }

    pub fn expr(&mut self, e: &Expr) -> Option<Ty> {
        match &e.kind {
            ExprKind::Int(_) => Some(Ty::Int),
            ExprKind::Float(_) => Some(Ty::Float),
            ExprKind::Str(_) => Some(Ty::Str),
            ExprKind::Bool(_) => Some(Ty::Bool),
            ExprKind::NoneLit => Some(Ty::None),
            ExprKind::Local(n) => self.locals.get(n).and_then(|s| s.known()),
            ExprKind::BinOp { op, left, right } => {
                let l = self.expr(left);
                let r = self.expr(right);
                let (l, r) = (l?, r?);
                if l == Ty::Obj || r == Ty::Obj {
                    return Some(Ty::Obj);
                }
                let lit = match right.kind {
                    ExprKind::Int(v) => Some(v),
                    _ => None,
                };
                let t = ops::binop(*op, l, r, lit);
                if t.is_none() {
                    self.report(
                        Diagnostic::error("P301", e.span, format!("unsupported operand type(s) for {}: '{l}' and '{r}'", op.symbol()))
                            .note("CPython would raise TypeError here"),
                    );
                }
                t
            }
            ExprKind::Unary { op, operand } => {
                let t = self.expr(operand)?;
                if t == Ty::Obj {
                    return Some(if *op == UnaryOp::Not { Ty::Bool } else { Ty::Obj });
                }
                let r = ops::unary(*op, t);
                if r.is_none() {
                    self.report(Diagnostic::error("P301", e.span, format!("bad operand type for unary operator: '{t}'")));
                }
                r
            }
            ExprKind::Compare { first, rest } => {
                let mut l = self.expr(first);
                for (op, c) in rest {
                    let r = self.expr(c);
                    if let (Some(a), Some(b)) = (l, r) {
                        if !ops::compare(*op, a, b) {
                            self.report(
                                Diagnostic::error("P301", c.span, format!("comparison not supported between instances of '{a}' and '{b}'"))
                                    .note("CPython would raise TypeError here"),
                            );
                        }
                    }
                    l = r;
                }
                // A single rich comparison on an object returns whatever the
                // object returns (e.g. a numpy array); chains reduce to bool.
                if let [(op, c)] = rest.as_slice() {
                    let ordering = !matches!(op, CmpOp::In | CmpOp::NotIn | CmpOp::Is | CmpOp::IsNot);
                    if ordering && (self.expr(first)? == Ty::Obj || self.expr(c)? == Ty::Obj) {
                        return Some(Ty::Obj);
                    }
                }
                Some(Ty::Bool)
            }
            ExprKind::BoolOp { op, values } => {
                if let Some(n) = short_circuit(self.locals, *op, values) {
                    // Statically decided: the result is operand n-1 itself.
                    let tys: Vec<Option<Ty>> = values[..n].iter().map(|v| self.expr(v)).collect();
                    return tys[n - 1];
                }
                let tys: Vec<Option<Ty>> = values.iter().map(|v| self.expr(v)).collect();
                let tys: Vec<Ty> = tys.into_iter().collect::<Option<_>>()?;
                self.merge(&tys, e, "`and`/`or`")
            }
            ExprKind::IfExp { test, body, orelse } => {
                self.expr(test);
                match static_truth(self.locals, test) {
                    Some(true) => return self.expr(body),
                    Some(false) => return self.expr(orelse),
                    None => {}
                }
                let a = self.expr(body);
                let b = self.expr(orelse);
                self.merge(&[a?, b?], e, "conditional expression")
            }
            ExprKind::ExternCall { ext, func, args } => {
                let tys: Vec<Option<Ty>> = args.iter().map(|a| self.expr(a)).collect();
                let tys: Vec<Ty> = tys.into_iter().collect::<Option<_>>()?;
                let m = &self.prog.module.externs[*ext];
                let f = &m.funcs[*func];
                if f.params.len() != tys.len() {
                    self.report(Diagnostic::error(
                        "P303",
                        e.span,
                        format!("{}.{}() takes {} argument(s) but {} were given", m.name, f.name, f.params.len(), tys.len()),
                    ));
                    return None;
                }
                for (i, (p, t)) in f.params.iter().zip(&tys).enumerate() {
                    if !p.accepts(ops::base(*t)) {
                        self.report(
                            Diagnostic::error("P303", args[i].span, format!("argument {} of {}.{}() expects {} (`{}`), got {t}", i + 1, m.name, f.name, p.ty(), p.rust()))
                                .note("CPython-side code would raise TypeError here"),
                        );
                    }
                }
                Some(f.ret.ty())
            }
            ExprKind::Annotated { ty, value } => {
                let v = self.expr(value)?;
                // Checked unbox (TypeError unless the object is exactly a `ty`).
                // `float` stays boxed: PEP 484 lets an int stand in for a float,
                // and converting it would change what the program prints.
                if v == Ty::Obj && matches!(ty, Ty::Int | Ty::Str | Ty::Bool) {
                    return Some(*ty);
                }
                if !crate::ops::satisfies(v, *ty) {
                    self.report(
                        Diagnostic::warning("P307", value.span, format!("{v} value where the annotation says {ty}"))
                            .note("CPython does not enforce annotations, so the value keeps its runtime type"),
                    );
                }
                Some(v)
            }
            ExprKind::Slot(_) | ExprKind::PyBuiltin(_) => Some(Ty::Obj),
            ExprKind::NewExc { args, .. } => {
                for a in args {
                    self.expr(a)?;
                }
                Some(Ty::exc())
            }
            ExprKind::GetAttr { obj, .. } => self.all_obj([&**obj]),
            ExprKind::CallObj { func, args, kwargs } => {
                let all: Vec<&Expr> = std::iter::once(&**func).chain(args.iter()).chain(kwargs.iter().map(|(_, v)| v)).collect();
                self.all_obj(all)
            }
            ExprKind::List(items) | ExprKind::Tuple(items) => self.all_obj(items.iter()),
            ExprKind::Dict(pairs) => {
                let all: Vec<&Expr> = pairs.iter().flat_map(|(k, v)| [k, v]).collect();
                self.all_obj(all)
            }
            ExprKind::Subscript { obj, index } => self.all_obj([&**obj, &**index]),
            ExprKind::Call { callee, args, .. } => {
                let tys: Vec<Option<Ty>> = args.iter().map(|a| self.expr(a)).collect();
                let tys: Option<Vec<Ty>> = tys.into_iter().collect();
                let tys = tys?;
                match callee {
                    Callee::Builtin(b) => match ops::builtin(*b, &tys, self.prog.boxing()) {
                        Ok(t) => Some(t),
                        Err(msg) => {
                            self.report(Diagnostic::error("P303", e.span, msg).note("CPython would raise TypeError here"));
                            None
                        }
                    },
                    Callee::Func(fid) => {
                        let f = &self.prog.module.funcs[*fid];
                        if f.params.len() != tys.len() {
                            self.report(Diagnostic::error(
                                "P303",
                                e.span,
                                format!("{}() takes {} positional argument(s) but {} were given", f.name, f.params.len(), tys.len()),
                            ));
                            return None;
                        }
                        let key = SpecKey { func: *fid, args: tys };
                        let r = self.prog.spec(&key).and_then(|s| self.prog.call_result(s));
                        if let Some(req) = self.requests.as_deref_mut() {
                            req.push((key, e.span));
                        }
                        r
                    }
                }
            }
        }
    }
}

fn contains_break(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match &s.kind {
        StmtKind::Break => true,
        StmtKind::If { body, orelse, .. } => contains_break(body) || contains_break(orelse),
        StmtKind::Try { body, handlers, orelse, finalbody } => {
            contains_break(body) || contains_break(orelse) || contains_break(finalbody) || handlers.iter().any(|h| contains_break(&h.body))
        }
        StmtKind::With { body, .. } => contains_break(body),
        _ => false, // breaks inside nested loops belong to those loops
    })
}
