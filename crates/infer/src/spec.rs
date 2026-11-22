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

use crate::{definite, native, ops};
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
    native::ty_join(a, b, false)
}

impl Slot {
    fn join(self, t: Ty, boxing: bool) -> Slot {
        match self {
            Slot::Unknown => Slot::Known(t),
            Slot::Known(a) if a == t => self,
            Slot::Known(a) if native::ty_join(a, t, boxing).is_some() => Slot::Known(native::ty_join(a, t, boxing).unwrap()),
            Slot::Known(a) if boxing || a == Ty::Obj || t == Ty::Obj => Slot::Boxed(a, t),
            Slot::Known(a) => Slot::Conflict(a, t),
            c => c,
        }
    }
    /// Join two slots.
    pub fn join_slot(self, o: Slot, boxing: bool) -> Slot {
        match o {
            Slot::Unknown => self,
            Slot::Known(t) => self.join(t, boxing),
            Slot::Boxed(a, b) => self.join(a, boxing).join(b, boxing),
            Slot::Conflict(..) => o,
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
    /// Lower bound on the return type requested by callers.
    pub ret_floor: Slot,
}

/// A module-level variable kept in a global cell: (unit, name).
pub type GlobalKey = (usize, String);

pub struct Program<'m> {
    pub module: &'m Module,
    pub mode: Mode,
    /// Types of global cells, joined over every assignment in the program.
    pub globals: BTreeMap<GlobalKey, Slot>,
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
        let mut ctx = self.ctx_for(spec);
        ctx.expr(e)
    }
}

pub fn specialize(module: &Module, mode: Mode) -> Program<'_> {
    let mut prog = Program { module, mode, globals: BTreeMap::new(), specs: vec![], index: HashMap::new(), diagnostics: vec![] };
    let main_key = SpecKey { func: module.main, args: vec![] };
    add_spec(&mut prog, main_key);

    let mut rounds = 0;
    loop {
        rounds += 1;
        let mut changed = false;
        let mut i = 0;
        while i < prog.specs.len() {
            let mut requests = Vec::new();
            let (locals, ret, falls, gassigns, ret_widen) = analyze(&prog, i, &mut requests, None);
            for (key, t) in ret_widen {
                let boxing = prog.boxing();
                if let Some(&j) = prog.index.get(&key) {
                    let floor = prog.specs[j].ret_floor.join(t, boxing);
                    if floor != prog.specs[j].ret_floor {
                        prog.specs[j].ret_floor = floor;
                        changed = true;
                    }
                }
            }
            for (key, t) in gassigns {
                let boxing = prog.boxing();
                let slot = prog.globals.entry(key).or_insert(Slot::Unknown);
                let joined = slot.join(t, boxing);
                if joined != *slot {
                    *slot = joined;
                    changed = true;
                }
            }
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
    prog.specs.push(Spec { id, key, mangled, ret: Slot::Unknown, falls_through: true, analyzed: false, locals, maybe_unbound, ret_floor: Slot::Unknown });
}

type Requests = Vec<(SpecKey, Span)>;

/// Analyse one specialization to a local fixed point under the current
/// return types of every other specialization.
fn analyze(
    prog: &Program,
    idx: usize,
    requests: &mut Requests,
    diags: Option<&mut Vec<Diagnostic>>,
) -> (BTreeMap<String, Slot>, Slot, bool, Vec<(GlobalKey, Ty)>, Vec<(SpecKey, Ty)>) {
    let spec = &prog.specs[idx];
    let mut gassigns;
    let mut ret_widen;
    let cur_ret = match spec.ret.join_slot(spec.ret_floor, prog.boxing()) {
        Slot::Known(t) => Some(t),
        Slot::Boxed(..) => Some(Ty::Obj),
        _ => None,
    };
    let f = &prog.module.funcs[spec.key.func];
    let mut locals = spec.locals.clone();
    let mut ret;
    let mut falls;
    let mut guard = 0;
    loop {
        let before = locals.clone();
        ret = spec.ret_floor;
        let mut ctx = Ctx { prog, locals: &locals, requests: Some(requests), diags: None, gassigns: vec![], widens: vec![], ret_widen: vec![], cur_ret };
        let mut assigns = Vec::new();
        falls = ctx.block(&f.body, &mut assigns, &mut ret);
        gassigns = std::mem::take(&mut ctx.gassigns);
        ret_widen = std::mem::take(&mut ctx.ret_widen);
        assigns.append(&mut ctx.widens);
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
        let mut ctx = Ctx { prog, locals: &locals, requests: None, diags: Some(d), gassigns: vec![], widens: vec![], ret_widen: vec![], cur_ret };
        ctx.block(&f.body, &mut sink, &mut ret2);
    }
    (locals, ret, falls, gassigns, ret_widen)
}

struct Ctx<'a, 'p> {
    prog: &'a Program<'p>,
    locals: &'a BTreeMap<String, Slot>,
    requests: Option<&'a mut Requests>,
    diags: Option<&'a mut Vec<Diagnostic>>,
    /// Assignments to global cells seen in this pass.
    gassigns: Vec<(GlobalKey, Ty)>,
    /// Places widened by uses (`xs.append(1)` widens `xs`), joined like assignments.
    widens: Vec<(String, Ty)>,
    /// Return types callers need from callees (`ys = make(); ys.append(1)`).
    ret_widen: Vec<(SpecKey, Ty)>,
    /// The return type of the function being analysed, as currently known.
    cur_ret: Option<Ty>,
}

/// How a `CallObj` node executes.
#[derive(Clone, Debug, PartialEq)]
pub enum CallClass {
    /// `recv.name(...)` on a native receiver.
    Method { recv: Ty, name: String },
    /// A CPython builtin with a native implementation for these argument types.
    Builtin(String),
    /// `isinstance(native, cls)`, decided at compile time.
    StaticBool(bool),
    /// Through the CPython bridge.
    Bridge,
    /// Not decidable yet (argument types still unknown in this pass).
    Pending,
}

/// Expressions that denote a storage location a value can be widened through.
fn is_place(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Local(_) | ExprKind::Global { .. } => true,
        ExprKind::Subscript { obj, index } => !matches!(index.kind, ExprKind::Slice { .. }) && is_place(obj),
        ExprKind::Call { callee: Callee::Func(_), .. } => true,
        ExprKind::CallObj { func, .. } => {
            matches!(&func.kind, ExprKind::GetAttr { obj, name } if matches!(name.as_str(), "setdefault" | "get" | "pop") && is_place(obj))
        }
        _ => false,
    }
}

/// `module.attr` name of a bridged callee, for the read-only whitelist.
fn callee_qualname(prog: &Program, f: &Expr) -> Option<String> {
    match &f.kind {
        ExprKind::PyBuiltin(n) => Some(n.clone()),
        ExprKind::Slot(i) => Some(match &prog.module.slots[*i].kind {
            SlotKind::Import { module, .. } => module.clone(),
            SlotKind::From { module, attr } => format!("{module}.{attr}"),
        }),
        ExprKind::GetAttr { obj, name } => callee_qualname(prog, obj).map(|q| format!("{q}.{name}")),
        _ => None,
    }
}

impl Ctx<'_, '_> {
    fn report(&mut self, d: Diagnostic) {
        if let Some(ds) = self.diags.as_deref_mut() {
            ds.push(d);
        }
    }

    fn boxing(&self) -> bool {
        self.prog.boxing()
    }

    fn local_ty(&self, n: &str) -> Option<Ty> {
        self.locals.get(n).and_then(|s| s.known())
    }

    /// Widen the type of a place (joined with its current type).
    fn widen(&mut self, place: &Expr, t: Ty) {
        match &place.kind {
            ExprKind::Local(n) => {
                if self.local_ty(n) != Some(t) {
                    self.widens.push((n.clone(), t));
                }
            }
            ExprKind::Global { unit, name } => self.gassigns.push(((*unit, name.clone()), t)),
            ExprKind::Subscript { obj, index } if !matches!(index.kind, ExprKind::Slice { .. }) => {
                let Some(ot) = self.expr(obj) else { return };
                match ot.as_comp() {
                    Some(Comp::List(e)) if e != t => self.widen(obj, Ty::list(t)),
                    Some(Comp::Dict(k, v)) if v != t => self.widen(obj, Ty::dict(k, t)),
                    _ => {}
                }
            }
            ExprKind::Call { callee: Callee::Func(fid), args, .. } => {
                let tys: Option<Vec<Ty>> = args.iter().map(|a| self.expr(a)).collect();
                if let Some(args) = tys {
                    self.ret_widen.push((SpecKey { func: *fid, args }, t));
                }
            }
            // Methods that return a stored element: `d.setdefault(k, []).append(x)`.
            ExprKind::CallObj { func, .. } => {
                if let ExprKind::GetAttr { obj, name } = &func.kind {
                    let Some(ot) = self.expr(obj) else { return };
                    match (ot.as_comp(), name.as_str()) {
                        (Some(Comp::Dict(k, v)), "setdefault" | "get" | "pop") if v != t => self.widen(obj, Ty::dict(k, t)),
                        (Some(Comp::List(e)), "pop") if e != t => self.widen(obj, Ty::list(t)),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// A value of type `vt` is stored where values have type `dest`: when it
    /// carries identity (a mutable container) its own place must agree.
    fn flow(&mut self, value: &Expr, vt: Ty, dest: Ty) {
        if vt != dest && vt.has_mutable() && is_place(value) {
            self.widen(value, dest);
        }
    }

    /// A value reaches arbitrary Python code: a mutable native container
    /// must be a Python object from creation (a copy would break aliasing).
    fn escape(&mut self, e: &Expr) {
        if let Some(t) = self.expr(e) {
            if t.has_mutable() && is_place(e) {
                self.widen(e, Ty::Obj);
            }
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
                    if let Some(dest) = self.local_ty(target) {
                        self.flow(value, t, dest);
                    }
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
                    Some(e) => {
                        let t = self.expr(e);
                        if let (Some(t), Some(dest)) = (t, self.cur_ret) {
                            self.flow(e, t, dest);
                        }
                        t
                    }
                    None => Some(Ty::None),
                };
                if let Some(t) = t {
                    *ret = ret.join(t, self.boxing());
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
                if let Some(it) = self.expr(iter) {
                    match native::iter_elem(it) {
                        Some(e) => {
                            assigns.push((var.clone(), e));
                            // `for row in grid: row.append(x)` widens grid's elements.
                            if let (Some(v), Some(Comp::List(_))) = (self.local_ty(var), it.as_comp()) {
                                if v != e && e.has_mutable() {
                                    self.widen(iter, Ty::list(v));
                                }
                            }
                        }
                        None => self.report(Diagnostic::error("P301", iter.span, format!("'{it}' object is not iterable")).note("CPython would raise TypeError here")),
                    }
                }
                self.block(body, assigns, ret);
            }
            StmtKind::SetItem { obj, index, value } => self.set_item(obj, index, value),
            StmtKind::SetAttr { obj, value, name } => {
                if let Some(ot) = self.expr(obj) {
                    if ot == Ty::Obj {
                        self.escape(value);
                    } else {
                        self.expr(value);
                        self.report(Diagnostic::error("P301", s.span, format!("'{ot}' object has no attribute '{name}'")).note("CPython would raise AttributeError here"));
                    }
                }
            }
            StmtKind::Del(target) => match &target.kind {
                ExprKind::Subscript { obj, index } => {
                    let ot = self.expr(obj);
                    self.index_types(index);
                    match ot.and_then(|t| t.as_comp()) {
                        Some(Comp::List(_) | Comp::Dict(..)) | None => {}
                        Some(_) => self.report(Diagnostic::error("P301", s.span, format!("'{}' object does not support item deletion", ot.unwrap()))),
                    }
                }
                ExprKind::GetAttr { obj, .. } => {
                    self.expr(obj);
                }
                _ => {}
            },
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
            StmtKind::AssignGlobal { unit, name, value } => {
                if let Some(t) = self.expr(value) {
                    self.gassigns.push(((*unit, name.clone()), t));
                    if let Some(dest) = self.prog.globals.get(&(*unit, name.clone())).and_then(|s| s.known()) {
                        self.flow(value, t, dest);
                    }
                }
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

    /// `obj[index] = value`
    fn set_item(&mut self, obj: &Expr, index: &Expr, value: &Expr) {
        let Some(ot) = self.expr(obj) else {
            self.expr(index);
            self.expr(value);
            return;
        };
        if ot == Ty::Obj {
            self.expr(index);
            self.escape(value);
            return;
        }
        let Some(vt) = self.expr(value) else { return };
        let boxing = self.boxing();
        let elem_join = |e: Ty, t: Ty| native::ty_join(e, t, boxing).or(boxing.then_some(Ty::Obj));
        match ot.as_comp() {
            Some(Comp::List(e)) => {
                if let ExprKind::Slice { .. } = index.kind {
                    self.index_types(index);
                    match native::iter_elem(vt).and_then(|x| elem_join(e, x)) {
                        Some(j) => self.widen(obj, Ty::list(j)),
                        None => self.report(Diagnostic::error("P301", value.span, "can only assign an iterable to a list slice")),
                    }
                    return;
                }
                let it = self.expr(index);
                if !it.map(ops::base).is_some_and(|t| matches!(t, Ty::Int | Ty::Bool | Ty::Obj)) {
                    self.report(Diagnostic::error("P301", index.span, "list indices must be integers or slices"));
                    return;
                }
                match elem_join(e, vt) {
                    Some(j) => {
                        if j != e {
                            self.widen(obj, Ty::list(j));
                        }
                        self.flow(value, vt, j);
                    }
                    None => self.report(Diagnostic::error("P304", value.span, format!("storing {vt} into a {ot} in native-only mode"))),
                }
            }
            Some(Comp::Dict(k, v)) => {
                let Some(kt) = self.expr(index) else { return };
                match (native::ty_join(k, kt, boxing).filter(|t| t.is_hashable()), elem_join(v, vt)) {
                    (Some(kk), Some(vv)) => {
                        if kk != k || vv != v {
                            self.widen(obj, Ty::dict(kk, vv));
                        }
                        self.flow(value, vt, vv);
                    }
                    _ => self.report(Diagnostic::error("P304", value.span, format!("storing [{kt}] = {vt} into a {ot} natively"))),
                }
            }
            _ => self.report(Diagnostic::error("P301", obj.span, format!("'{ot}' object does not support item assignment")).note("CPython would raise TypeError here")),
        }
    }

    /// Type-check the parts of an index or slice.
    fn index_types(&mut self, index: &Expr) {
        if let ExprKind::Slice { lo, hi, step } = &index.kind {
            for p in [lo, hi, step].into_iter().flatten() {
                if let Some(t) = self.expr(p) {
                    if !matches!(ops::base(t), Ty::Int | Ty::Bool | Ty::None | Ty::Obj) {
                        self.report(Diagnostic::error("P301", p.span, "slice indices must be integers or None"));
                    }
                }
            }
        } else {
            self.expr(index);
        }
    }

    /// Common type of values flowing into one result (and/or, if-else).
    fn merge(&mut self, tys: &[Ty], e: &Expr, what: &str) -> Option<Ty> {
        let first = tys[0];
        if tys.iter().all(|t| *t == first) {
            return Some(first);
        }
        if let Some(j) = tys[1..].iter().try_fold(first, |acc, t| native::ty_join(acc, *t, self.boxing())) {
            return Some(j);
        }
        if self.boxing() || tys.contains(&Ty::Obj) {
            return Some(Ty::Obj);
        }
        let names: Vec<&str> = tys.iter().map(|t| t.py_name()).collect();
        self.report(
            Diagnostic::error("P302", e.span, format!("{what} yields a value-dependent type ({})", names.join(" or ")))
                .note("native-only mode cannot box; convert explicitly (e.g. bool(...)) or use hybrid mode"),
        );
        None
    }

    /// Element type of a container literal, with flows from its items.
    fn elements(&mut self, items: &[Expr], e: &Expr, what: &str) -> Option<Ty> {
        let tys: Vec<Option<Ty>> = items.iter().map(|x| self.expr(x)).collect();
        let tys: Vec<Ty> = tys.into_iter().collect::<Option<_>>()?;
        let j = match native::join_all(tys.iter().copied(), self.boxing()) {
            Some(j) => j,
            None => {
                let names: Vec<&str> = tys.iter().map(|t| t.py_name()).collect();
                self.report(
                    Diagnostic::error("P302", e.span, format!("{what} mixes element types ({})", names.join(", ")))
                        .note("native-only mode cannot box; use one element type or hybrid mode"),
                );
                return None;
            }
        };
        for (x, t) in items.iter().zip(&tys) {
            self.flow(x, *t, j);
        }
        Some(j)
    }

    /// Classify a `CallObj` node (method / native builtin / bridge).
    fn classify(&mut self, func: &Expr, args: &[Expr], kwargs: &[(String, Expr)]) -> CallClass {
        match &func.kind {
            ExprKind::GetAttr { obj, name } => match self.expr(obj) {
                Some(t) if t != Ty::Obj => {
                    let argtys: Option<Vec<Ty>> = args.iter().map(|a| self.expr(a)).collect();
                    let kwtys: Option<Vec<(String, Ty)>> = kwargs.iter().map(|(k, v)| self.expr(v).map(|t| (k.clone(), t))).collect();
                    match (argtys, kwtys) {
                        (Some(a), Some(k)) if native::method(ops::base(t), name, &a, &k, self.boxing()).is_some() => {
                            CallClass::Method { recv: ops::base(t), name: name.clone() }
                        }
                        (Some(_), Some(_)) => CallClass::Bridge,
                        _ => CallClass::Pending,
                    }
                }
                Some(_) => CallClass::Bridge,
                None => CallClass::Pending,
            },
            ExprKind::PyBuiltin(name) => {
                let argtys: Option<Vec<Ty>> = args.iter().map(|a| self.expr(a)).collect();
                let kwtys: Option<Vec<(String, Ty)>> = kwargs.iter().map(|(k, v)| self.expr(v).map(|t| (k.clone(), t))).collect();
                let (Some(a), Some(k)) = (argtys, kwtys) else { return CallClass::Pending };
                if name == "isinstance" && a.len() == 2 {
                    let classes: Option<Vec<&str>> = match &args[1].kind {
                        ExprKind::PyBuiltin(c) => Some(vec![c.as_str()]),
                        ExprKind::Tuple(items) => items.iter().map(|i| if let ExprKind::PyBuiltin(c) = &i.kind { Some(c.as_str()) } else { None }).collect(),
                        _ => None,
                    };
                    if let Some(cs) = classes {
                        let rs: Option<Vec<bool>> = cs.iter().map(|c| native::isinstance_static(a[0], c)).collect();
                        if let Some(rs) = rs {
                            return CallClass::StaticBool(rs.iter().any(|r| *r));
                        }
                    }
                    return CallClass::Bridge;
                }
                match native::builtin(name, &a, &k, self.boxing()) {
                    Some(_) => CallClass::Builtin(name.clone()),
                    None => CallClass::Bridge,
                }
            }
            _ => CallClass::Bridge,
        }
    }

    pub fn expr(&mut self, e: &Expr) -> Option<Ty> {
        match &e.kind {
            ExprKind::Int(_) => Some(Ty::Int),
            ExprKind::Float(_) => Some(Ty::Float),
            ExprKind::Str(_) => Some(Ty::Str),
            ExprKind::Bool(_) => Some(Ty::Bool),
            ExprKind::NoneLit => Some(Ty::None),
            ExprKind::Local(n) => self.local_ty(n),
            ExprKind::Global { unit, name } => self.prog.globals.get(&(*unit, name.clone())).and_then(|s| s.known()),
            ExprKind::BinOp { op, left, right } => {
                let l = self.expr(left);
                let r = self.expr(right);
                let (l, r) = (l?, r?);
                if l.is_never() || r.is_never() {
                    return Some(Ty::never());
                }
                if *op == BinOp::Mod && ops::base(l) == Ty::Str {
                    // printf-style formatting; a dict right operand (`%(name)s`) uses CPython.
                    if r == Ty::Obj || r.dict_kv().is_some() {
                        self.escape(right);
                        return Some(Ty::Obj);
                    }
                    return Some(Ty::Str);
                }
                if l == Ty::Obj || r == Ty::Obj {
                    self.escape(left);
                    self.escape(right);
                    return Some(Ty::Obj);
                }
                if l.as_comp().is_some_and(|c| !matches!(c, Comp::Opt(_))) || r.as_comp().is_some_and(|c| !matches!(c, Comp::Opt(_))) {
                    let t = native::coll_binop(*op, ops::base(l), ops::base(r), self.boxing());
                    if t.is_none() {
                        self.report(
                            Diagnostic::error("P301", e.span, format!("unsupported operand type(s) for {}: '{l}' and '{r}'", op.symbol()))
                                .note("CPython would raise TypeError here"),
                        );
                    }
                    return t;
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
                if t.is_never() {
                    return Some(Ty::never());
                }
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
                let mut prev = &**first;
                for (op, c) in rest {
                    let r = self.expr(c);
                    if let (Some(a), Some(b)) = (l, r) {
                        if a == Ty::Obj || b == Ty::Obj {
                            self.escape(prev);
                            self.escape(c);
                        } else if !a.is_never() && !b.is_never() && !ops::compare(*op, a, b) {
                            self.report(
                                Diagnostic::error("P301", c.span, format!("comparison not supported between instances of '{a}' and '{b}'"))
                                    .note("CPython would raise TypeError here"),
                            );
                        }
                    }
                    l = r;
                    prev = c;
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
            ExprKind::GetAttr { obj, name } => {
                let t = self.expr(obj)?;
                if t == Ty::Obj {
                    return Some(Ty::Obj);
                }
                if !t.has_mutable() {
                    return Some(Ty::Obj); // e.g. `x.real`: boxed copy of an immutable value
                }
                self.report(
                    Diagnostic::error("P301", e.span, format!("attribute `{name}` of a native {t} used as a value"))
                        .note("call the method directly (`xs.append(v)`); bound methods are not first-class natively"),
                );
                None
            }
            ExprKind::CallObj { func, args, kwargs } => self.call_obj(e, func, args, kwargs),
            ExprKind::List(items) => {
                let j = self.elements(items, e, "list literal")?;
                Some(Ty::list(j))
            }
            ExprKind::Tuple(items) => {
                let tys: Vec<Option<Ty>> = items.iter().map(|x| self.expr(x)).collect();
                Some(Ty::tuple(tys.into_iter().collect::<Option<_>>()?))
            }
            ExprKind::Set(items) => {
                let j = self.elements(items, e, "set literal")?;
                if j.is_hashable() {
                    Some(Ty::set(j))
                } else if self.boxing() {
                    Some(Ty::Obj)
                } else {
                    self.report(Diagnostic::error("P302", e.span, format!("set of {j} is not native (elements must be hashable natively)")));
                    None
                }
            }
            ExprKind::Dict(pairs) => {
                let keys: Vec<Expr> = pairs.iter().map(|(k, _)| k.clone()).collect();
                let vals: Vec<Expr> = pairs.iter().map(|(_, v)| v.clone()).collect();
                let k = self.elements(&keys, e, "dict literal keys")?;
                let v = self.elements(&vals, e, "dict literal values")?;
                if k.is_hashable() {
                    Some(Ty::dict(k, v))
                } else if self.boxing() {
                    for x in &vals {
                        self.escape(x);
                    }
                    Some(Ty::Obj)
                } else {
                    self.report(Diagnostic::error("P302", e.span, format!("dict with {k} keys is not native (keys must be hashable natively)")));
                    None
                }
            }
            ExprKind::Slice { lo, hi, step } => {
                for p in [lo, hi, step].into_iter().flatten() {
                    self.expr(p)?;
                }
                Some(Ty::Obj)
            }
            ExprKind::Starred(inner) => {
                let t = self.expr(inner)?;
                if native::iter_elem(t).is_none() && t.dict_kv().is_none() {
                    self.report(Diagnostic::error("P301", e.span, format!("argument after * must be an iterable, not {t}")).note("CPython would raise TypeError here"));
                    return None;
                }
                Some(t)
            }
            ExprKind::Unpack { value, n, star } => {
                let vt = self.expr(value)?;
                let n = *n;
                let unpack_err = |got: usize| -> String {
                    match star {
                        Some(_) => format!("not enough values to unpack (expected at least {}, got {got})", n - 1),
                        None if got > n => format!("too many values to unpack (expected {n})"),
                        None => format!("not enough values to unpack (expected {n}, got {got})"),
                    }
                };
                if let Some(ts) = vt.tuple_items() {
                    let ok = match star {
                        None => ts.len() == n,
                        Some(_) => ts.len() + 1 >= n,
                    };
                    if !ok {
                        self.report(Diagnostic::error("P301", e.span, unpack_err(ts.len())).note("CPython would raise ValueError here"));
                        return None;
                    }
                    return Some(match star {
                        None => vt,
                        Some(k) => {
                            let after = n - 1 - k;
                            let mid = &ts[*k..ts.len() - after];
                            let me = native::join_all(mid.iter().copied(), self.boxing())?;
                            let mut items: Vec<Ty> = ts[..*k].to_vec();
                            items.push(Ty::list(me));
                            items.extend_from_slice(&ts[ts.len() - after..]);
                            Ty::tuple(items)
                        }
                    });
                }
                let Some(el) = native::iter_elem(vt) else {
                    self.report(Diagnostic::error("P301", e.span, format!("cannot unpack non-iterable {vt} object")).note("CPython would raise TypeError here"));
                    return None;
                };
                let items: Vec<Ty> = (0..n).map(|i| if Some(i) == *star { Ty::list(el) } else { el }).collect();
                Some(Ty::tuple(items))
            }
            ExprKind::Comp { kind, elt, val, gens } => {
                for g in gens {
                    let it = self.expr(&g.iter)?;
                    match native::iter_elem(it) {
                        Some(el) => self.widens.push((g.var.clone(), el)),
                        None => {
                            self.report(Diagnostic::error("P301", g.iter.span, format!("'{it}' object is not iterable")).note("CPython would raise TypeError here"));
                            return None;
                        }
                    }
                    let mut assigns = Vec::new();
                    let mut ret = Slot::Unknown;
                    for st in &g.unpack {
                        self.stmt(st, &mut assigns, &mut ret);
                    }
                    self.widens.append(&mut assigns);
                    for c in &g.ifs {
                        self.expr(c);
                    }
                }
                let et = self.expr(elt)?;
                match kind {
                    CompKind::List | CompKind::Gen => Some(Ty::list(et)),
                    CompKind::Set if et.is_hashable() => Some(Ty::set(et)),
                    CompKind::Dict => {
                        let vt = self.expr(val.as_deref().unwrap())?;
                        if et.is_hashable() {
                            Some(Ty::dict(et, vt))
                        } else {
                            self.report(Diagnostic::error("P302", e.span, format!("dict comprehension with {et} keys is not native")));
                            None
                        }
                    }
                    CompKind::Set => {
                        self.report(Diagnostic::error("P302", e.span, format!("set comprehension of {et} is not native")));
                        None
                    }
                }
            }
            ExprKind::Subscript { obj, index } => {
                let ot = self.expr(obj)?;
                if ot.is_never() {
                    return Some(Ty::never());
                }
                if ot == Ty::Obj {
                    self.index_types(index);
                    self.escape(index);
                    return Some(Ty::Obj);
                }
                let lit = |x: &Option<Box<Expr>>| -> Option<Option<i64>> {
                    match x.as_deref().map(|e| &e.kind) {
                        None => Some(None),
                        Some(ExprKind::Int(v)) => Some(Some(*v)),
                        _ => None,
                    }
                };
                let (idx, it, ilit, slits) = match &index.kind {
                    ExprKind::Slice { lo, hi, step } => {
                        self.index_types(index);
                        let sl = match (lit(lo), lit(hi), lit(step)) {
                            (Some(a), Some(b), Some(c)) => Some([a, b, c]),
                            _ => None,
                        };
                        (native::Index::Slice, None, None, sl)
                    }
                    k => {
                        let it = self.expr(index);
                        let ilit = if let ExprKind::Int(v) = k { Some(*v) } else { None };
                        (native::Index::Item, it, ilit, None)
                    }
                };
                match native::subscript(ops::base(ot), idx, it, ilit, slits) {
                    Ok(t) => Some(t),
                    Err(msg) => {
                        self.report(Diagnostic::error("P301", e.span, msg).note("CPython would raise TypeError/IndexError here"));
                        None
                    }
                }
            }
            ExprKind::Call { callee, args, .. } => {
                let tys: Vec<Option<Ty>> = args.iter().map(|a| self.expr(a)).collect();
                let tys: Option<Vec<Ty>> = tys.into_iter().collect();
                let tys = tys?;
                match callee {
                    Callee::Builtin(b) => match ops::builtin(*b, &tys, self.boxing()) {
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
                        let key = SpecKey { func: *fid, args: tys.clone() };
                        let callee = self.prog.spec(&key);
                        let r = callee.and_then(|s| self.prog.call_result(s));
                        // A callee that widens a parameter (appends to it, lets it
                        // escape) widens the caller's argument place too.
                        if let Some(s) = callee.filter(|s| s.analyzed) {
                            let params: Vec<Option<Ty>> = f.params.iter().map(|p| s.locals.get(p).and_then(|x| x.known())).collect();
                            for ((a, t), pt) in args.iter().zip(&tys).zip(params) {
                                if let Some(pt) = pt {
                                    self.flow(a, *t, pt);
                                }
                            }
                        }
                        if let Some(req) = self.requests.as_deref_mut() {
                            req.push((key, e.span));
                        }
                        r
                    }
                }
            }
        }
    }

    fn call_obj(&mut self, e: &Expr, func: &Expr, args: &[Expr], kwargs: &[(String, Expr)]) -> Option<Ty> {
        match self.classify(func, args, kwargs) {
            CallClass::Pending => {
                // Evaluate the parts (requests, flows) without deciding yet.
                for a in args.iter().chain(kwargs.iter().map(|(_, v)| v)) {
                    self.expr(a);
                }
                None
            }
            CallClass::StaticBool(_) => Some(Ty::Bool),
            CallClass::Method { recv, name } => {
                let ExprKind::GetAttr { obj, .. } = &func.kind else { unreachable!() };
                let a: Vec<Ty> = args.iter().map(|x| self.expr(x)).collect::<Option<_>>()?;
                let k: Vec<(String, Ty)> = kwargs.iter().map(|(n, v)| self.expr(v).map(|t| (n.clone(), t))).collect::<Option<_>>()?;
                match native::method(recv, &name, &a, &k, self.boxing())? {
                    Ok(m) => {
                        if let Some(w) = m.widen {
                            self.widen(obj, w);
                            // Values stored into the container share its element type.
                            let elem = w.list_elem().or(w.set_elem()).or(w.dict_kv().map(|kv| kv.1));
                            if let Some(el) = elem {
                                if let Some((x, t)) = args.iter().zip(&a).last() {
                                    self.flow(x, *t, el);
                                }
                            }
                        }
                        Some(m.ret)
                    }
                    Err(msg) => {
                        self.report(Diagnostic::error("P301", e.span, msg).note("CPython would raise AttributeError/TypeError here"));
                        None
                    }
                }
            }
            CallClass::Builtin(name) => {
                let a: Vec<Ty> = args.iter().map(|x| self.expr(x)).collect::<Option<_>>()?;
                let k: Vec<(String, Ty)> = kwargs.iter().map(|(n, v)| self.expr(v).map(|t| (n.clone(), t))).collect::<Option<_>>()?;
                match native::builtin(&name, &a, &k, self.boxing())? {
                    Ok(t) => Some(t),
                    Err(msg) => {
                        self.report(Diagnostic::error("P303", e.span, msg).note("CPython would raise TypeError/ValueError here"));
                        None
                    }
                }
            }
            CallClass::Bridge => {
                // A native receiver whose method has no native version is boxed (immutable only).
                if let ExprKind::GetAttr { obj, .. } = &func.kind {
                    let t = self.expr(obj)?;
                    if t != Ty::Obj && t.has_mutable() {
                        self.escape(obj);
                    }
                } else {
                    self.expr(func)?;
                }
                let read_only = callee_qualname(self.prog, func).is_some_and(|q| native::READ_ONLY_CALLS.contains(&q.as_str()));
                let mut ok = true;
                for a in args.iter().chain(kwargs.iter().map(|(_, v)| v)) {
                    if read_only {
                        ok &= self.expr(a).is_some();
                    } else {
                        self.escape(a);
                        ok &= self.expr(a).is_some();
                    }
                }
                ok.then_some(Ty::Obj)
            }
        }
    }
}

impl<'m> Program<'m> {
    /// How a `CallObj` node executes in `spec` (shared by analysis and codegen).
    pub fn classify_call(&self, spec: &Spec, func: &Expr, args: &[Expr], kwargs: &[(String, Expr)]) -> CallClass {
        let mut ctx = self.ctx_for(spec);
        ctx.classify(func, args, kwargs)
    }

    fn ctx_for<'a>(&'a self, spec: &'a Spec) -> Ctx<'a, 'm> {
        Ctx {
            prog: self,
            locals: &spec.locals,
            requests: None,
            diags: None,
            gassigns: vec![],
            widens: vec![],
            ret_widen: vec![],
            cur_ret: None,
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
