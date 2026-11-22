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
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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
    /// Generators: what `send` passes (from `.send(v)` calls on its values).
    pub gen_send: Slot,
    /// Generators: what the body returns (`return v`; `None` falling off the end).
    pub gen_ret: Slot,
    /// Generators: whether the body can fall off its end.
    pub gen_falls: bool,
    /// The specializations its latest analysis calls (resolved keys).
    pub calls: Vec<SpecKey>,
    /// Reads of `T | None` locals (by span) where every path has excluded
    /// `None` (`if x is None: continue`, `if x is not None: ...`): typed `T`
    /// and unwrapped by codegen.
    pub narrowed: BTreeSet<Span>,
}

/// A module-level variable kept in a global cell: (unit, name).
pub type GlobalKey = (usize, String);

pub struct Program<'m> {
    pub module: &'m Module,
    pub mode: Mode,
    /// Types of global cells, joined over every assignment in the program.
    pub globals: BTreeMap<GlobalKey, Slot>,
    /// Types of instance attributes of native classes, joined program-wide.
    pub fields: BTreeMap<(ClassId, String), Slot>,
    /// Classes instantiated anywhere.
    pub instantiated: BTreeSet<ClassId>,
    /// Function values that reach CPython (wrapped as Python callables).
    pub boxed_fns: BTreeSet<Ty>,
    /// Native classes whose instances reach CPython (wrapped as `native_object`).
    pub boxed_classes: BTreeSet<ClassId>,
    pub specs: Vec<Spec>,
    pub index: HashMap<SpecKey, usize>,
    pub diagnostics: Vec<Diagnostic>,
    /// Diagnostics found in the body of a specialization (its index). Only
    /// those of specializations reachable under the final types are reported
    /// (`analysis::plan`): early rounds can create specializations for types
    /// that later widen.
    pub spec_diagnostics: Vec<(usize, Diagnostic)>,
    /// Specializations no longer reachable from the entry module under the
    /// current types (recomputed every round). They don't count towards the
    /// per-function limit: as a parameter type widens (a record gaining keys),
    /// each round leaves a stale specialization behind.
    pub dead: HashSet<usize>,
    /// Number of specializations when the current round started. The limit
    /// counts only those (and only live ones): a specialization created
    /// during a round may be stale by the next one, and falling back to the
    /// shared object-typed specialization too early widens field and record
    /// types for good (joins never narrow).
    pub round_start: usize,
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
        // A generator: its slot holds the join of what it yields.
        if self.module.funcs[s.key.func].is_generator {
            let y = match s.ret {
                Slot::Known(t) | Slot::Conflict(t, _) => t,
                Slot::Boxed(..) => Ty::Obj,
                Slot::Unknown => Ty::never(),
            };
            if self.module.funcs[s.key.func].is_async {
                return Some(Ty::agen2(y, gen_send_ty(s)));
            }
            return Some(Ty::gen3(y, gen_send_ty(s), gen_ret_ty(s)));
        }
        let r = match (s.ret, s.falls_through) {
            (Slot::Known(Ty::None), _) | (Slot::Unknown, true) => Some(Ty::None),
            // Never returns normally (always raises): `Never`.
            (Slot::Unknown, false) if !returns_value(&self.module.funcs[s.key.func].body) => Some(Ty::never()),
            (Slot::Known(t), false) => Some(t),
            // Falling off the end returns None: `T | None`.
            (Slot::Known(t), true) => Some(opt_join(t, Ty::None).unwrap_or(Ty::Obj)),
            (Slot::Boxed(..), _) => Some(Ty::Obj),
            _ => None,
        };
        // Calling an `async def` makes a coroutine; its body produces `r`.
        if self.module.funcs[s.key.func].is_async {
            return r.map(Ty::coro);
        }
        r
    }

    /// What the body of `s` returns (the call result, minus the coroutine of
    /// an `async def`).
    pub fn body_result(&self, s: &Spec) -> Option<Ty> {
        let r = self.call_result(s)?;
        Some(if self.module.funcs[s.key.func].is_async { r.as_coro().unwrap_or(r) } else { r })
    }

    /// The specialization a direct call with argument types `key` uses: its
    /// own, or (hybrid mode, once the function has reached the limit) the
    /// shared one with object-typed immutable arguments.
    /// Specializations of `func` that are reachable (or not analysed yet).
    pub fn live_count(&self, func: FuncId) -> usize {
        self.specs[..self.round_start].iter().filter(|s| s.key.func == func && !self.dead.contains(&s.id)).count()
    }

    /// Recompute `dead`: analysed specializations not reachable from the
    /// entry module through the calls of the latest analyses.
    fn compute_dead(&mut self) {
        let mut seen = vec![false; self.specs.len()];
        let mut stack = vec![0usize];
        // Specializations not analysed yet have no calls recorded: keep them
        // and what they may reach alive by treating them as roots.
        stack.extend(self.specs.iter().filter(|s| !s.analyzed).map(|s| s.id));
        while let Some(i) = stack.pop() {
            if seen[i] {
                continue;
            }
            seen[i] = true;
            for k in &self.specs[i].calls {
                if let Some(&j) = self.index.get(k) {
                    stack.push(j);
                }
            }
        }
        self.dead = (0..self.specs.len()).filter(|&i| !seen[i]).collect();
    }

    pub fn resolve_key(&self, key: SpecKey) -> SpecKey {
        if self.index.contains_key(&key) || !self.boxing() {
            return key;
        }
        if self.live_count(key.func) < MAX_SPECS_PER_FUNCTION {
            return key;
        }
        generic_key(&self.module.funcs[key.func], &key)
    }

    /// `math.NAME` (`import math`) or `NAME` (`from math import NAME`): the
    /// name in the `math` module (when that import is the stdlib's).
    pub fn math_name(&self, e: &Expr) -> Option<&'static str> {
        let (slot, name) = match &e.kind {
            ExprKind::GetAttr { obj, name } => match obj.kind {
                ExprKind::Slot(i) => (i, name.as_str()),
                _ => return None,
            },
            ExprKind::Slot(i) => match &self.module.slots[*i].kind {
                SlotKind::From { module, attr } if module == "math" => return Some(pyfast_ir::types::intern_str(attr.as_str())),
                _ => return None,
            },
            _ => return None,
        };
        match &self.module.slots[slot].kind {
            SlotKind::Import { module, .. } if module == "math" => Some(pyfast_ir::types::intern_str(name)),
            _ => None,
        }
    }

    /// `math.pi` etc.: the Rust constant.
    pub fn math_const_of(&self, e: &Expr) -> Option<&'static str> {
        self.math_name(e).and_then(native::math_const)
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
            // `if x:` on a local that is exactly `None` in this specialization.
            ExprKind::Local(n) if locals.get(n).and_then(|s| s.known()) == Some(Ty::None) => Some(false),
            ExprKind::BoolOp { op, values } => {
                // Decided when the operands before the deciding one are all
                // decided (they are side-effect-free tests): `x is not None and
                // f(x)` is false when `x` is always None, without calling `f`.
                let and = *op == BoolOpKind::And;
                for v in values {
                    match static_truth(locals, v)? {
                        false if and => return Some(false),
                        true if !and => return Some(true),
                        _ => {}
                    }
                }
                Some(and)
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
    let mut prog = Program {
        module,
        mode,
        globals: BTreeMap::new(),
        fields: BTreeMap::new(),
        instantiated: BTreeSet::new(),
        boxed_fns: BTreeSet::new(),
        boxed_classes: BTreeSet::new(),
        specs: vec![],
        index: HashMap::new(),
        diagnostics: vec![],
        spec_diagnostics: vec![],
        dead: HashSet::new(),
        round_start: 0,
    };
    let main_key = SpecKey { func: module.main, args: vec![] };
    add_spec(&mut prog, main_key);

    let mut rounds = 0;
    loop {
        rounds += 1;
        let mut changed = false;
        prog.compute_dead();
        prog.round_start = prog.specs.len();
        let mut i = 0;
        while i < prog.specs.len() {
            let mut requests = Vec::new();
            let (locals, ret, falls, gassigns, ret_widen, Extra { fassigns, news, gen_ret, send_widen, gen_falls, narrowed }) = analyze(&prog, i, &mut requests, None);
            prog.specs[i].gen_falls = gen_falls;
            if prog.specs[i].narrowed != narrowed {
                prog.specs[i].narrowed = narrowed;
                changed = true;
            }
            let calls: Vec<SpecKey> = requests.iter().map(|(k, _)| prog.resolve_key(k.clone())).collect();
            if prog.specs[i].calls != calls {
                prog.specs[i].calls = calls;
                changed = true;
            }
            for (key, t) in send_widen {
                if let Some(&j) = prog.index.get(&key) {
                    let floor = prog.specs[j].gen_send.join(t, prog.boxing());
                    if floor != prog.specs[j].gen_send {
                        prog.specs[j].gen_send = floor;
                        changed = true;
                    }
                }
            }
            if prog.specs[i].gen_ret != gen_ret {
                prog.specs[i].gen_ret = gen_ret;
                changed = true;
            }
            for (key, t) in fassigns {
                let boxing = prog.boxing();
                let slot = prog.fields.entry(key).or_insert(Slot::Unknown);
                let joined = slot.join(t, boxing);
                if joined != *slot {
                    *slot = joined;
                    changed = true;
                }
            }
            // Instances seen by CPython share their mutable fields with it:
            // those fields are Python objects (re-applied as fields gain types).
            let boxing = prog.boxing();
            let keys: Vec<(ClassId, String)> = prog.fields.keys().filter(|(c, _)| prog.boxed_classes.contains(c)).cloned().collect();
            for key in keys {
                let slot = prog.fields[&key];
                if slot.known().is_some_and(|t| t.has_mutable() && t != Ty::Obj) {
                    let joined = slot.join(Ty::Obj, boxing);
                    if joined != slot {
                        prog.fields.insert(key, joined);
                        changed = true;
                    }
                }
            }
            for c in news {
                match c.1 {
                    Some(t) if t.as_closure().is_some() => changed |= prog.boxed_fns.insert(t),
                    Some(_) => changed |= prog.boxed_classes.insert(c.0),
                    None => changed |= prog.instantiated.insert(c.0),
                }
            }
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
                    let count = prog.live_count(key.func);
                    // Hybrid mode: past the limit, calls share one object-typed
                    // specialization (`Program::resolve_key`), which is always allowed.
                    let g = generic_key(&module.funcs[key.func], &key);
                    let generic = prog.boxing() && g == key;
                    // Requested in the same round as others that reached the limit:
                    // the caller resolves to the shared specialization next round.
                    if count >= MAX_SPECS_PER_FUNCTION && !generic && prog.boxing() {
                        if !prog.index.contains_key(&g) {
                            add_spec(&mut prog, g);
                        }
                        changed = true;
                        continue;
                    }
                    if count >= MAX_SPECS_PER_FUNCTION && !generic {
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
    let mut owners: Vec<usize> = Vec::new();
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
                // A generator's slot is what it yields (nothing: Never).
                (Slot::Unknown, _) if f.is_generator => (Some(Slot::Known(Ty::never())), true),
                (Slot::Known(t), _) if f.is_generator => (Some(Slot::Known(t)), true),
                (Slot::Known(t), false) => (Some(Slot::Known(t)), true),
                (Slot::Known(Ty::None), true) => (Some(Slot::Known(Ty::None)), true),
                (Slot::Unknown, true) => (Some(Slot::Known(Ty::None)), true),
                (Slot::Unknown, false) if !returns_value(&f.body) => (Some(Slot::Known(Ty::never())), true),
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
        owners.resize(diags.len(), i);
    }
    let mut seen = HashSet::new();
    for (d, i) in diags.into_iter().zip(owners) {
        if seen.insert((d.code, d.span, d.message.clone())) {
            prog.spec_diagnostics.push((i, d));
        }
    }
    prog
}

/// Whether a function body has a `return <value>` anywhere (a body without
/// one that cannot fall off its end always raises).
fn returns_value(stmts: &[Stmt]) -> bool {
    if std::env::var("PF_NO_NEVER").is_ok() { return true; }
    stmts.iter().any(|s| match &s.kind {
        StmtKind::Return(Some(_)) => true,
        StmtKind::If { body, orelse, .. } => returns_value(body) || returns_value(orelse),
        StmtKind::While { body, .. } | StmtKind::ForRange { body, .. } | StmtKind::ForIter { body, .. } | StmtKind::With { body, .. } => returns_value(body),
        StmtKind::Try { body, handlers, orelse, finalbody } => returns_value(body) || handlers.iter().any(|h| returns_value(&h.body)) || returns_value(orelse) || returns_value(finalbody),
        _ => false,
    })
}

/// `key` with every immutable argument boxed (the environment of a closure
/// and mutable containers keep their types: boxing would copy them).
pub fn generic_key(f: &Function, key: &SpecKey) -> SpecKey {
    let off = env_offset(f);
    let args = key.args.iter().enumerate().map(|(i, t)| if i < off || t.has_mutable() { *t } else { Ty::Obj }).collect();
    SpecKey { func: key.func, args }
}


pub fn spec_label(prog: &Program, s: &Spec) -> String {
    let f = &prog.module.funcs[s.key.func];
    if f.is_module {
        return if f.unit == 0 { "<module>".into() } else { format!("<module {}>", prog.module.units[f.unit].name) };
    }
    format!("{}({})", f.name, s.key.args.iter().skip(env_offset(f)).map(|t| t.py_name()).collect::<Vec<_>>().join(", "))
}

/// 1 when a function's specializations take a closure environment first
/// (nested functions and lambdas), else 0.
pub fn env_offset(f: &Function) -> usize {
    usize::from(f.parent.is_some())
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
    let off = env_offset(f);
    for (p, t) in f.params.iter().zip(key.args.iter().skip(off)) {
        locals.insert(p.clone(), Slot::Known(*t));
    }
    // A nested function's environment carries its captured variables' types.
    if off == 1 {
        if let Some((_, caps)) = key.args.first().and_then(|t| t.as_closure()) {
            for (n, t) in f.captures.iter().zip(caps) {
                locals.insert(n.clone(), Slot::Known(t));
            }
        }
    }
    // Functions defined in module-level blocks (`if c: def f` / `else: def f`)
    // share a name and have no enclosing function to tell them apart.
    let mangled = if prog.specs.iter().any(|s| s.mangled == mangled && s.key.func != key.func) {
        mangled.replacen("__", &format!("_f{}__", key.func), 1)
    } else {
        mangled
    };
    let maybe_unbound = definite::maybe_unbound(f);
    let id = prog.specs.len();
    prog.index.insert(key.clone(), id);
    prog.specs.push(Spec { id, key, mangled, ret: Slot::Unknown, falls_through: true, analyzed: false, locals, maybe_unbound, ret_floor: Slot::Unknown, gen_send: Slot::Unknown, gen_ret: Slot::Unknown, gen_falls: true, calls: vec![], narrowed: BTreeSet::new() });
}

type Requests = Vec<(SpecKey, Span)>;

/// Analyse one specialization to a local fixed point under the current
/// return types of every other specialization.
fn analyze(
    prog: &Program,
    idx: usize,
    requests: &mut Requests,
    diags: Option<&mut Vec<Diagnostic>>,
) -> (BTreeMap<String, Slot>, Slot, bool, Vec<(GlobalKey, Ty)>, Vec<(SpecKey, Ty)>, Extra) {
    let spec = &prog.specs[idx];
    let mut gassigns;
    let mut ret_widen;
    let mut fassigns;
    let mut news_acc: Vec<(ClassId, Option<Ty>)> = Vec::new();
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
    let cur_send = gen_send_ty(spec);
    let mut gen_ret;
    let mut send_widen;
    let mut gen_falls = true;
    let mut narrowed;
    loop {
        let before = locals.clone();
        ret = spec.ret_floor;
        gen_ret = Slot::Unknown;
        let mut ctx = Ctx { prog, locals: &locals, requests: Some(requests), diags: None, gassigns: vec![], widens: vec![], ret_widen: vec![], cur_ret, fassigns: vec![], news: vec![], generator: f.is_generator, yields: vec![], cur_send, gen_returns: vec![], send_widen: vec![], nn: Some(BTreeSet::new()), nn_reads: vec![], narrowed: None };
        let mut assigns = Vec::new();
        falls = ctx.block(&f.body, &mut assigns, &mut ret);
        // A generator's "return type" is what it yields; what it returns is
        // `gen_ret` (`None` when it can fall off the end).
        if f.is_generator {
            for t in std::mem::take(&mut ctx.yields) {
                ret = ret.join(t, prog.boxing());
            }
            gen_falls = falls;
            if falls {
                ctx.gen_returns.push(Ty::None);
            }
            for t in std::mem::take(&mut ctx.gen_returns) {
                gen_ret = gen_ret.join(t, prog.boxing());
            }
            falls = true;
        }
        send_widen = std::mem::take(&mut ctx.send_widen);
        narrowed = narrowed_reads(std::mem::take(&mut ctx.nn_reads));
        gassigns = std::mem::take(&mut ctx.gassigns);
        ret_widen = std::mem::take(&mut ctx.ret_widen);
        fassigns = std::mem::take(&mut ctx.fassigns);
        // Escapes are monotone but their trigger can vanish once the place is
        // widened (a list boxed to `object`): keep what every pass found.
        news_acc.append(&mut ctx.news);
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
        let mut ctx = Ctx { prog, locals: &locals, requests: None, diags: Some(d), gassigns: vec![], widens: vec![], ret_widen: vec![], cur_ret, fassigns: vec![], news: vec![], generator: f.is_generator, yields: vec![], cur_send, gen_returns: vec![], send_widen: vec![], nn: Some(BTreeSet::new()), nn_reads: vec![], narrowed: None };
        ctx.block(&f.body, &mut sink, &mut ret2);
    }
    (locals, ret, falls, gassigns, ret_widen, Extra { fassigns, news: news_acc, gen_ret, send_widen, gen_falls, narrowed })
}

/// Read sites narrowed on every visit.
fn narrowed_reads(reads: Vec<(Span, bool)>) -> BTreeSet<Span> {
    let no: BTreeSet<Span> = reads.iter().filter(|r| !r.1).map(|r| r.0).collect();
    reads.into_iter().filter(|r| r.1 && !no.contains(&r.0)).map(|r| r.0).collect()
}

/// Locals a statement list may assign (loop variables, `with`/`except`
/// targets included), at any depth: they lose their narrowing around loops
/// and `try`.
fn assigned_locals(stmts: &[Stmt], out: &mut BTreeSet<String>) {
    for s in stmts {
        match &s.kind {
            StmtKind::Assign { target, .. } | StmtKind::AugAssign { target, .. } => {
                out.insert(target.clone());
            }
            StmtKind::If { body, orelse, .. } => {
                assigned_locals(body, out);
                assigned_locals(orelse, out);
            }
            StmtKind::While { body, .. } => assigned_locals(body, out),
            StmtKind::ForRange { var, body, .. } | StmtKind::ForIter { var, body, .. } => {
                out.insert(var.clone());
                assigned_locals(body, out);
            }
            StmtKind::Try { body, handlers, orelse, finalbody } => {
                assigned_locals(body, out);
                for h in handlers {
                    if let Some(n) = &h.name {
                        out.insert(n.clone());
                    }
                    assigned_locals(&h.body, out);
                }
                assigned_locals(orelse, out);
                assigned_locals(finalbody, out);
            }
            StmtKind::With { var, body, .. } => {
                if let Some(v) = var {
                    out.insert(v.clone());
                }
                assigned_locals(body, out);
            }
            _ => {}
        }
    }
}

/// Whether control never reaches the end of `stmts` (it ends in `return`,
/// `raise`, `break` or `continue`, or an `if` whose branches all do).
fn block_exits(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match &s.kind {
        StmtKind::Return(_) | StmtKind::Raise(_) | StmtKind::Break | StmtKind::Continue => true,
        StmtKind::If { body, orelse, .. } => block_exits(body) && block_exits(orelse),
        _ => false,
    })
}

/// The locals a test proves not `None` when it is true, and when it is false.
fn test_facts(e: &Expr) -> (Vec<String>, Vec<String>) {
    match &e.kind {
        ExprKind::Compare { first, rest } if rest.len() == 1 && matches!(rest[0].0, CmpOp::Is | CmpOp::IsNot) => {
            let var = match (&first.kind, &rest[0].1.kind) {
                (ExprKind::Local(n), ExprKind::NoneLit) | (ExprKind::NoneLit, ExprKind::Local(n)) => n.clone(),
                _ => return (vec![], vec![]),
            };
            if rest[0].0 == CmpOp::Is { (vec![], vec![var]) } else { (vec![var], vec![]) }
        }
        // `if x:` (None is falsy).
        ExprKind::Local(n) => (vec![n.clone()], vec![]),
        ExprKind::Unary { op: UnaryOp::Not, operand } => {
            let (t, f) = test_facts(operand);
            (f, t)
        }
        ExprKind::BoolOp { op: BoolOpKind::And, values } => (values.iter().flat_map(|v| test_facts(v).0).collect(), vec![]),
        ExprKind::BoolOp { op: BoolOpKind::Or, values } => (vec![], values.iter().flat_map(|v| test_facts(v).1).collect()),
        _ => (vec![], vec![]),
    }
}

/// More results of `analyze`.
struct Extra {
    fassigns: Vec<((ClassId, String), Ty)>,
    news: Vec<(ClassId, Option<Ty>)>,
    gen_ret: Slot,
    send_widen: Vec<(SpecKey, Ty)>,
    gen_falls: bool,
    narrowed: BTreeSet<Span>,
}

/// What a generator's `yield` expressions evaluate to: `None` (from `next`)
/// or what `send` passes.
pub fn gen_send_ty(s: &Spec) -> Ty {
    match s.gen_send {
        Slot::Known(t) => native::ty_join(Ty::None, t, true).unwrap_or(Ty::Obj),
        Slot::Boxed(..) => Ty::Obj,
        _ => Ty::None,
    }
}

/// What a generator returns (`return v`; `None` when it falls off the end).
pub fn gen_ret_ty(s: &Spec) -> Ty {
    match s.gen_ret {
        Slot::Known(t) => t,
        Slot::Boxed(..) => Ty::Obj,
        // Not known yet (or never returns): the bottom type, absorbed by joins.
        _ => Ty::never(),
    }
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
    /// Instance-attribute assignments seen in this pass.
    fassigns: Vec<((ClassId, String), Ty)>,
    /// Classes instantiated in this pass, and (with `Some`) function values
    /// that escape to CPython.
    news: Vec<(ClassId, Option<Ty>)>,
    /// Analysing a generator body: `return` ends the iteration.
    generator: bool,
    /// Types of the values yielded in this pass.
    yields: Vec<Ty>,
    /// The type of `yield` expressions in this generator (what `send` passes).
    cur_send: Ty,
    /// Types of `return` values in this generator.
    gen_returns: Vec<Ty>,
    /// Send types callers need from generator functions (`g.send(v)`).
    send_widen: Vec<(SpecKey, Ty)>,
    /// Flow-sensitive analysis (inference passes): the `T | None` locals
    /// known not to be `None` at the current point.
    nn: Option<BTreeSet<String>>,
    /// Reads of optional locals seen in this pass, and whether each was narrowed.
    nn_reads: Vec<(Span, bool)>,
    /// Outside a flow pass (codegen's `type_of`): the narrowed read sites.
    narrowed: Option<&'a BTreeSet<Span>>,
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
    /// A method of a natively compiled class.
    UserMethod { func: FuncId, kind: MethodKind },
    /// `cls(...)` on a class object value.
    Construct(ClassId),
    /// A method call on a union of native classes (dispatched per variant).
    UnionMethod(String),
    /// `isinstance(u, C)` on a union: decided by the variant at run time.
    RuntimeIsinstance(Vec<ClassId>),
    /// A call of a function value (closure, lambda, function used as a value).
    ClosureCall,
    /// A method of a Rust/C struct handle (`counter.incr(2)`).
    ExtMethod { ext: usize, func: usize },
    /// `record.get("key"[, default])` with a key of the record (always present).
    RecordGet(&'static str),
    /// A function of the `math` module on native numbers (`math.sqrt(x)`).
    Math(&'static str),
}

/// A native instance or a union of them.
pub fn class_like(t: Ty) -> bool {
    t.class_id().is_some() || t.union_members().is_some()
}

/// `f(*args, **kwargs)` forwarding: a starred fixed-shape tuple expands into
/// its items, and an empty `**kwargs` dict forwards nothing (so wrappers can
/// call functions with fixed parameters).
pub fn expand_stars(ty: &mut dyn FnMut(&Expr) -> Option<Ty>, args: &[Expr], kwargs: &[(String, Expr)]) -> (Vec<Expr>, Vec<(String, Expr)>) {
    let simple = |e: &Expr| matches!(e.kind, ExprKind::Local(_) | ExprKind::Global { .. } | ExprKind::Tuple(_));
    let mut out = Vec::new();
    for a in args {
        if let ExprKind::Starred(inner) = &a.kind {
            if let Some(items) = ty(inner).and_then(|t| t.tuple_items()) {
                if simple(inner) {
                    for i in 0..items.len() {
                        out.push(Expr {
                            kind: ExprKind::Subscript { obj: inner.clone(), index: Box::new(Expr { kind: ExprKind::Int(i as i64), span: a.span }) },
                            span: a.span,
                        });
                    }
                    continue;
                }
            }
        }
        out.push(a.clone());
    }
    let kw: Vec<(String, Expr)> = kwargs
        .iter()
        .filter(|(k, v)| !(k == "**" && ty(v).and_then(|t| t.dict_kv()).is_some_and(|(k, _)| k.is_never())))
        .cloned()
        .collect();
    (out, kw)
}

/// Defaults stored in a closure's environment are read from the called value.
pub fn env_defaults(bound: Vec<Expr>, func: &Expr) -> Vec<Expr> {
    bound
        .into_iter()
        .map(|a| match a.kind {
            ExprKind::EnvDefault(index) => Expr { kind: ExprKind::EnvRead { value: Box::new(func.clone()), index }, span: a.span },
            _ => a,
        })
        .collect()
}

/// `x` in `type(x)`.
pub fn type_call_arg(e: &Expr) -> Option<&Expr> {
    match &e.kind {
        ExprKind::CallObj { func, args, kwargs } if matches!(&func.kind, ExprKind::PyBuiltin(n) if n == "type") && args.len() == 1 && kwargs.is_empty() => Some(&args[0]),
        _ => None,
    }
}

/// Whether a type contains an instance of a native class.
pub fn has_class(t: Ty) -> bool {
    match t.as_comp() {
        Some(Comp::Class(..) | Comp::Union(_)) => true,
        Some(Comp::List(x) | Comp::Set(x) | Comp::Opt(x) | Comp::TupleVar(x)) => has_class(x),
        Some(Comp::Dict(k, v) | Comp::View(_, k, v)) => has_class(k) || has_class(v),
        Some(Comp::Tuple(ts)) => ts.iter().any(|t| has_class(*t)),
        _ => false,
    }
}

/// Native classes appearing in a type.
pub fn classes_in(t: Ty) -> Vec<ClassId> {
    match t.as_comp() {
        Some(Comp::Class(c, _)) => vec![c],
        Some(Comp::Union(ms)) => ms.iter().flat_map(|m| classes_in(*m)).collect(),
        Some(Comp::List(x) | Comp::Set(x) | Comp::Opt(x) | Comp::TupleVar(x)) => classes_in(x),
        Some(Comp::Dict(k, v) | Comp::View(_, k, v)) => classes_in(k).into_iter().chain(classes_in(v)).collect(),
        Some(Comp::Tuple(ts)) => ts.iter().flat_map(|t| classes_in(*t)).collect(),
        _ => vec![],
    }
}

/// `+` -> `add` etc. (dunder stems).
pub fn dunder_name(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "add",
        BinOp::Sub => "sub",
        BinOp::Mul => "mul",
        BinOp::Div => "truediv",
        BinOp::FloorDiv => "floordiv",
        BinOp::Mod => "mod",
        BinOp::Pow => "pow",
        BinOp::BitAnd => "and",
        BinOp::BitOr => "or",
        BinOp::BitXor => "xor",
        BinOp::LShift => "lshift",
        BinOp::RShift => "rshift",
        BinOp::MatMul => "matmul",
    }
}

/// Dunder name of a comparison (`==` -> `__eq__`) and of its reflection.
pub fn cmp_dunder(op: CmpOp) -> Option<(&'static str, &'static str)> {
    Some(match op {
        CmpOp::Eq => ("__eq__", "__eq__"),
        CmpOp::NotEq => ("__ne__", "__ne__"),
        CmpOp::Lt => ("__lt__", "__gt__"),
        CmpOp::LtE => ("__le__", "__ge__"),
        CmpOp::Gt => ("__gt__", "__lt__"),
        CmpOp::GtE => ("__ge__", "__le__"),
        _ => return None,
    })
}

/// Expressions that denote a storage location a value can be widened through.
fn is_place(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Local(_) | ExprKind::Global { .. } => true,
        ExprKind::GetAttr { obj, .. } => is_place(obj),
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

    /// A read of local `n`: a `T | None` local is `T` where `None` has been
    /// excluded on every path to this read.
    fn local_read(&mut self, n: &str, sp: Span) -> Option<Ty> {
        let t = self.local_ty(n)?;
        let Some(inner) = t.as_opt() else { return Some(t) };
        let narrowed = match &self.nn {
            Some(set) => {
                let y = set.contains(n);
                self.nn_reads.push((sp, y));
                y
            }
            None => self.narrowed.is_some_and(|s| s.contains(&sp)),
        };
        Some(if narrowed { inner } else { t })
    }

    /// After `target = <t>`: narrowed when `t` can't be `None`.
    fn nn_assign(&mut self, target: &str, t: Option<Ty>) {
        if let Some(set) = &mut self.nn {
            match t {
                Some(t) if t != Ty::None && t != Ty::Obj && t.as_opt().is_none() => set.insert(target.to_string()),
                _ => set.remove(target),
            };
        }
    }

    fn nn_add(&mut self, names: &[String]) {
        if let Some(set) = &mut self.nn {
            set.extend(names.iter().cloned());
        }
    }

    fn nn_kill(&mut self, names: &BTreeSet<String>) {
        if let Some(set) = &mut self.nn {
            set.retain(|n| !names.contains(n));
        }
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
                    Some(Comp::Record(fs)) => match &index.kind {
                        ExprKind::Str(k) => match record_with(&fs, k, t) {
                            Some(r) if r != ot => self.widen(obj, r),
                            Some(_) => {}
                            None => {
                                let d = self.degrade(obj, ot);
                                if let Some((k, _)) = d.dict_kv() {
                                    self.widen(obj, Ty::dict(k, t));
                                }
                            }
                        },
                        _ => {
                            let d = self.degrade(obj, ot);
                            if let Some((k, _)) = d.dict_kv() {
                                self.widen(obj, Ty::dict(k, t));
                            }
                        }
                    },
                    _ => {}
                }
            }
            ExprKind::Call { callee: Callee::Func(fid), args, .. } => {
                let tys: Option<Vec<Ty>> = args.iter().map(|a| self.expr(a)).collect();
                // An `async def` call is a coroutine (a generator call a
                // generator): widen what its body returns (yields).
                let callee = &self.prog.module.funcs[*fid];
                let t = if callee.is_generator && callee.is_async {
                    match t.as_agen2() {
                        Some((y, s)) => {
                            // What callers pass to `asend`.
                            if let Some(args) = tys.clone() {
                                if s != Ty::None {
                                    self.send_widen.push((SpecKey { func: *fid, args }, s));
                                }
                            }
                            y
                        }
                        None => return,
                    }
                } else if callee.is_generator {
                    match t.as_gen() {
                        Some((y, s, _)) => {
                            // What callers send to it (`g.send(v)`).
                            if let Some(args) = tys.clone() {
                                if s != Ty::None {
                                    self.send_widen.push((SpecKey { func: *fid, args }, s));
                                }
                            }
                            y
                        }
                        None => return,
                    }
                } else if callee.is_async {
                    match t.as_coro() {
                        Some(r) => r,
                        None => return,
                    }
                } else {
                    t
                };
                if let Some(args) = tys {
                    self.ret_widen.push((SpecKey { func: *fid, args }, t));
                }
            }
            ExprKind::GetAttr { obj, name } => {
                if let Some(c) = self.expr(obj).and_then(|t| t.class_id()) {
                    if self.prog.fields.get(&(c, name.clone())).and_then(|s| s.known()) != Some(t) {
                        self.fassigns.push(((c, name.clone()), t));
                    }
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
        // A function or instance stored where a Python object is kept (a
        // variable that also holds a CPython value): it is boxed at the
        // store, so it needs its Python wrapper like any escaping value.
        if dest == Ty::Obj && vt != Ty::Obj {
            let members = vt.union_members().unwrap_or(vec![vt]);
            if has_class(vt) || members.iter().any(|m| m.as_closure().is_some()) {
                self.escape(value);
                return;
            }
        }
        if vt != dest && vt.has_mutable() && is_place(value) {
            self.widen(value, dest);
        }
    }

    /// A record dict used like a dict (other methods, computed keys,
    /// iteration): its place becomes a plain `dict[str, V]`; returns that type.
    fn degrade(&mut self, obj: &Expr, t: Ty) -> Ty {
        match native::record_dict(t, self.boxing()) {
            Some(d) => {
                if is_place(obj) {
                    self.widen(obj, d);
                }
                d
            }
            None => t,
        }
    }

    /// The type a builtin sees for an argument: records are dicts, except to
    /// the builtins implemented on records directly.
    fn record_arg(&mut self, builtin: &str, arg: &Expr, t: Ty) -> Ty {
        if t.as_record().is_none() || RECORD_BUILTINS.contains(&builtin) || builtin == "isinstance" {
            return native::record_dict(t, self.boxing()).filter(|_| builtin == "isinstance").unwrap_or(t);
        }
        self.degrade(arg, t)
    }

    /// A value reaches arbitrary Python code: a mutable native container
    /// must be a Python object from creation (a copy would break aliasing).
    fn escape(&mut self, e: &Expr) {
        if let Some(t) = self.expr(e) {
            // A function value becomes a Python callable over an all-`object`
            // specialization (its cells stay shared with native callers).
            let fns: Vec<Ty> = t.union_members().unwrap_or(vec![t]).into_iter().filter(|m| m.as_closure().is_some()).collect();
            if !fns.is_empty() && fns.len() == t.union_members().map(|m| m.len()).unwrap_or(1) {
                for m in fns {
                    let (fid, _) = m.as_closure().unwrap();
                    let f = &self.prog.module.funcs[fid];
                    let mut args: Vec<Ty> = if env_offset(f) == 1 { vec![m] } else { vec![] };
                    args.extend(f.sig.names.iter().map(|_| Ty::Obj));
                    if f.sig.vararg.is_some() {
                        args.push(Ty::tuple_var(Ty::Obj));
                    }
                    if f.sig.kwarg.is_some() {
                        args.push(Ty::dict(Ty::Str, Ty::Obj));
                    }
                    if let Some(req) = self.requests.as_deref_mut() {
                        req.push((SpecKey { func: fid, args }, e.span));
                    }
                    self.news.push((0, Some(m)));
                }
                return;
            }
            if t.as_ext_struct().is_some() {
                self.report(
                    Diagnostic::error("P231", e.span, format!("a handle to the Rust/C struct `{t}` is passed to Python"))
                        .note("handles stay native; pass their fields (plain values) to Python instead"),
                );
                return;
            }
            if has_class(t) {
                // Instances become `native_object`s: their methods get
                // all-`object` specializations; fields holding mutable
                // containers become Python objects (shared with Python).
                for c in classes_in(t) {
                    self.box_class(e, c);
                }
                if t.class_id().is_some() || t.union_members().is_some() {
                    return;
                }
            }
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
                let vt = self.expr(value);
                if let Some(t) = vt {
                    assigns.push((target.clone(), t));
                    if let Some(dest) = self.local_ty(target) {
                        self.flow(value, t, dest);
                    }
                }
                self.nn_assign(target, vt);
            }
            StmtKind::AugAssign { target, op, value } => {
                let cur = Expr { kind: ExprKind::Local(target.clone()), span: s.span };
                let e = Expr {
                    kind: ExprKind::BinOp { op: *op, left: Box::new(cur), right: Box::new(value.clone()) },
                    span: s.span,
                };
                let vt = self.expr(&e);
                if let Some(t) = vt {
                    assigns.push((target.clone(), t));
                }
                self.nn_assign(target, vt);
            }
            StmtKind::Expr(e) => {
                self.expr(e);
            }
            // `return v` ends a generator; `v` is what `yield from` produces.
            StmtKind::Return(v) if self.generator => {
                let t = match v {
                    Some(e) => self.expr(e),
                    None => Some(Ty::None),
                };
                if let Some(t) = t {
                    self.gen_returns.push(t);
                }
                return false;
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
                // `None` narrowing: each branch starts with what the test
                // proves; afterwards, what holds at the end of every branch
                // that can reach the next statement.
                let (tf, ff) = test_facts(test);
                let start = self.nn.clone();
                self.nn_add(&tf);
                let a = self.block(body, assigns, ret);
                let end_a = std::mem::replace(&mut self.nn, start.clone());
                self.nn_add(&ff);
                let b = self.block(orelse, assigns, ret);
                let end_b = self.nn.take();
                let (exit_a, exit_b) = (!a || block_exits(body), !b || block_exits(orelse));
                self.nn = match (exit_a, exit_b) {
                    (true, true) => start,
                    (true, false) => end_b,
                    (false, true) => end_a,
                    (false, false) => match (end_a, end_b) {
                        (Some(x), Some(y)) => Some(x.intersection(&y).cloned().collect()),
                        _ => None,
                    },
                };
                return a || b;
            }
            StmtKind::While { test, body } => {
                // Locals assigned in the loop may hold `None` again at the test.
                let mut kill = BTreeSet::new();
                assigned_locals(body, &mut kill);
                self.nn_kill(&kill);
                self.expr(test);
                if static_truth(self.locals, test) == Some(false) {
                    return true;
                }
                let start = self.nn.clone();
                self.nn_add(&test_facts(test).0);
                self.block(body, assigns, ret);
                self.nn = start;
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
                let mut kill = BTreeSet::from([var.clone()]);
                assigned_locals(body, &mut kill);
                self.nn_kill(&kill);
                let start = self.nn.clone();
                self.block(body, assigns, ret);
                self.nn = start;
            }
            StmtKind::ForIter { var, iter, body, is_async: true } if self.expr(iter).is_some_and(|t| t.as_agen().is_some()) => {
                let y = self.expr(iter).and_then(|t| t.as_agen()).unwrap();
                assigns.push((var.clone(), y));
                let mut kill = BTreeSet::from([var.clone()]);
                assigned_locals(body, &mut kill);
                self.nn_kill(&kill);
                let start = self.nn.clone();
                self.block(body, assigns, ret);
                self.nn = start;
            }
            StmtKind::ForIter { var, iter, body, is_async: true } => {
                // A native instance is iterated through its Python wrapper.
                if self.expr(iter).is_some_and(class_like) {
                    self.escape(iter);
                } else if let Some(t) = self.expr(iter).filter(|t| *t != Ty::Obj) {
                    self.report(Diagnostic::error("P301", iter.span, format!("'async for' requires an object with __aiter__ method, got {t}")).note("CPython would raise TypeError here"));
                }
                assigns.push((var.clone(), Ty::Obj));
                let mut kill = BTreeSet::from([var.clone()]);
                assigned_locals(body, &mut kill);
                self.nn_kill(&kill);
                let start = self.nn.clone();
                self.block(body, assigns, ret);
                self.nn = start;
            }
            StmtKind::ForIter { var, iter, body, .. } => {
                if let Some(t) = self.expr(iter).filter(|t| t.as_record().is_some()) {
                    self.degrade(iter, t);
                }
                let it = match self.expr(iter).map(|t| native::record_dict(t, self.boxing()).unwrap_or(t)) {
                    Some(t) if class_like(t) => match self.dunder(iter, t, "__iter__", &[]) {
                        Some(r) => r,
                        None => {
                            self.report(Diagnostic::error("P301", iter.span, format!("'{t}' object is not iterable")));
                            None
                        }
                    },
                    t => t,
                };
                if let Some(it) = it {
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
                let mut kill = BTreeSet::from([var.clone()]);
                assigned_locals(body, &mut kill);
                self.nn_kill(&kill);
                let start = self.nn.clone();
                if let Some(e) = it.and_then(native::iter_elem) {
                    self.nn_assign(var, Some(e));
                }
                self.block(body, assigns, ret);
                self.nn = start;
            }
            StmtKind::SetItem { obj, index, value } => self.set_item(obj, index, value),
            StmtKind::SetAttr { obj, value, name } if self.expr(obj).is_some_and(|t| t.as_ext_struct().is_some()) => {
                let (ext, si) = self.expr(obj).unwrap().as_ext_struct().unwrap();
                let st = &self.prog.module.externs[ext].structs[si];
                let field = st.fields.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone());
                let sname = st.name.clone();
                if let Some(vt) = self.expr(value) {
                    match field {
                        Some(ft) if ft.accepts(vt) => {}
                        Some(ft) => self.report(Diagnostic::error("P303", value.span, format!("field `{name}` of `{sname}` expects {}, got {vt}", ft.ty()))),
                        None => self.report(Diagnostic::error("P301", s.span, format!("'{sname}' has no public field '{name}'"))),
                    }
                }
            }
            StmtKind::SetAttr { obj, value, name } => {
                if let Some(ot) = self.expr(obj) {
                    if let Some(ms) = ot.union_members() {
                        if let Some(vt) = self.expr(value) {
                            for m in ms {
                                self.fassigns.push(((m.class_id().unwrap(), name.clone()), vt));
                            }
                        }
                        return true;
                    }
                    if let Some(c) = ot.class_id() {
                        if let Some(m) = self.prog.module.find_method(c, name).cloned() {
                            if m.kind == MethodKind::Property {
                                // `@x.setter` is a separate method with the same name.
                                let setter = self.prog.module.mro(c).into_iter().find_map(|k| {
                                    self.prog.module.class(k).methods.iter().find(|mm| mm.name == *name && mm.kind == MethodKind::Setter).cloned()
                                });
                                match setter {
                                    Some(st) => {
                                        self.call_spec(&Expr { kind: ExprKind::NoneLit, span: s.span }, st.func, &[ot], std::slice::from_ref(value));
                                    }
                                    None => self.report(Diagnostic::error("P301", s.span, format!("property '{name}' of '{}' object has no setter", self.prog.module.class(c).name))),
                                }
                                return true;
                            }
                        }
                        if let Some(vt) = self.expr(value) {
                            self.fassigns.push(((c, name.clone()), vt));
                            if let Some(dest) = self.prog.fields.get(&(c, name.clone())).and_then(|x| x.known()) {
                                self.flow(value, vt, dest);
                            }
                        }
                        return true;
                    }
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
                    // Deleting an added (optional) key keeps the record.
                    let optional_key = |t: Ty| matches!(&index.kind, ExprKind::Str(k) if t.as_record().is_some_and(|fs| fs.iter().any(|f| f.key == k.as_str() && f.optional)));
                    let ot = self.expr(obj).map(|t| if t.as_record().is_some() && !optional_key(t) { self.degrade(obj, t) } else { t });
                    if ot.is_some_and(|t| t.as_record().is_some()) {
                        return true;
                    }
                    self.index_types(index);
                    match ot.and_then(|t| t.as_comp()) {
                        Some(Comp::List(_) | Comp::Dict(..)) | None => {}
                        Some(_) => self.report(Diagnostic::error("P301", s.span, format!("'{}' object does not support item deletion", ot.unwrap()))),
                    }
                }
                ExprKind::GetAttr { obj, .. } => {
                    self.expr(obj);
                }
                ExprKind::Local(n) => {
                    self.expr(target);
                    if let Some(set) = &mut self.nn {
                        set.remove(n);
                    }
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
                let entry = self.nn.clone();
                let mut falls = self.block(body, assigns, ret) && self.block(orelse, assigns, ret);
                // A handler or `finally` may start anywhere in the body.
                let mut kill = BTreeSet::new();
                assigned_locals(body, &mut kill);
                assigned_locals(orelse, &mut kill);
                for h in handlers {
                    if let Some(n) = &h.name {
                        kill.insert(n.clone());
                    }
                    assigned_locals(&h.body, &mut kill);
                }
                self.nn = entry;
                self.nn_kill(&kill);
                for h in handlers {
                    for t in &h.types {
                        if let ExcType::Obj(e) = t {
                            self.expr(e);
                        }
                    }
                    if let Some(n) = &h.name {
                        // `except MyError as e` binds the native instance.
                        let t = match h.types.as_slice() {
                            [ExcType::Native(c)] => self.class_ty(*c),
                            _ => Ty::exc(),
                        };
                        assigns.push((n.clone(), t));
                    }
                    falls |= self.block(&h.body, assigns, ret);
                }
                // A body that may raise reaches the handlers; one that never
                // raises still falls through when its own end is reachable.
                let mut fkill = BTreeSet::new();
                for h in handlers {
                    assigned_locals(&h.body, &mut fkill);
                }
                self.nn_kill(&fkill);
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
                        let native_exc = t.class_id().is_some_and(|c| self.prog.module.exception_base(c).is_some());
                        if t != Ty::Obj && t != Ty::exc() && !native_exc {
                            self.report(Diagnostic::error("P301", e.span, "exceptions must derive from BaseException").note("CPython would raise TypeError here"));
                        }
                    }
                }
                return false;
            }
            StmtKind::With { ctx, var, body, is_async } => {
                if let Some(t) = self.expr(ctx) {
                    if *is_async && class_like(t) {
                        // `async with` on a native instance: through its Python wrapper.
                        self.escape(ctx);
                    } else if t != Ty::Obj {
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
                let mut kill = var.iter().cloned().collect::<BTreeSet<String>>();
                assigned_locals(body, &mut kill);
                let entry = self.nn.clone();
                self.block(body, assigns, ret);
                self.nn = entry;
                self.nn_kill(&kill);
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
        if class_like(ot) {
            if self.dunder(obj, ot, "__setitem__", &[index, value]).is_none() {
                self.report(Diagnostic::error("P301", obj.span, format!("'{ot}' object does not support item assignment")));
            }
            return;
        }
        if ot == Ty::Obj {
            self.expr(index);
            self.escape(value);
            return;
        }
        let Some(vt) = self.expr(value) else { return };
        let boxing = self.boxing();
        let elem_join = |e: Ty, t: Ty| native::ty_join(e, t, boxing).or(boxing.then_some(Ty::Obj));
        if let Some(fs) = ot.as_record() {
            // A literal key: the field's type joins the value's (a new key is
            // an optional field).
            if let ExprKind::Str(k) = &index.kind {
                let cur = fs.iter().find(|f| f.key == k.as_str()).map(|f| f.ty);
                let joined = cur.map(|c| elem_join(c, vt)).unwrap_or(Some(vt));
                if let Some((j, r)) = joined.and_then(|j| record_with(&fs, k, j).map(|r| (j, r))) {
                    if r != ot {
                        self.widen(obj, r);
                    }
                    self.flow(value, vt, j);
                    return;
                }
                if !boxing && joined.is_none() {
                    self.report(Diagnostic::error("P304", value.span, format!("storing {vt} into key {k:?} of {ot} natively")));
                    return;
                }
            }
            // A computed key: the record is a plain dict.
            let it = self.expr(index);
            let d = self.degrade(obj, ot);
            if let (Some((k, v)), Some(it)) = (d.dict_kv(), it) {
                if let (Some(kk), Some(vv)) = (native::ty_join(k, it, boxing).filter(|t| t.is_hashable()), elem_join(v, vt)) {
                    self.widen(obj, Ty::dict(kk, vv));
                } else if boxing {
                    self.widen(obj, Ty::Obj);
                }
            }
            return;
        }
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
                    // Hybrid: keys that are not natively hashable (Python
                    // objects) make the dict a Python dict.
                    _ if boxing => self.widen(obj, Ty::Obj),
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
        // `math.sqrt(x)` on native numbers.
        if let Some(name) = self.prog.math_name(func) {
            if !args.iter().any(|a| matches!(a.kind, ExprKind::Starred(_))) && kwargs.iter().all(|(k, _)| k != "**") {
                let a: Option<Vec<Ty>> = args.iter().map(|x| self.expr(x).map(ops::base)).collect();
                let k: Option<Vec<(String, Ty)>> = kwargs.iter().map(|(n, v)| self.expr(v).map(|t| (n.clone(), ops::base(t)))).collect();
                match (a, k) {
                    (Some(a), Some(k)) => {
                        if native::math_call(name, &a, &k).is_some() {
                            return CallClass::Math(name);
                        }
                    }
                    _ => return CallClass::Pending,
                }
            }
        }
        if let ExprKind::GetAttr { obj, name } = &func.kind {
            if let Some(t) = self.expr(obj) {
                if let Some((ext, si)) = t.as_ext_struct() {
                    let m = &self.prog.module.externs[ext];
                    return match m.funcs.iter().position(|f| f.owner == Some(si) && f.recv.is_some() && f.short() == name) {
                        Some(func) => CallClass::ExtMethod { ext, func },
                        None => CallClass::Bridge,
                    };
                }
                if t.union_members().is_some() {
                    return CallClass::UnionMethod(name.clone());
                }
                let class = t.class_id().or(match t.as_comp() {
                    Some(Comp::Type(c, _)) => Some(c),
                    _ => None,
                });
                if let Some(c) = class {
                    return match self.prog.module.find_method(c, name) {
                        Some(m) => CallClass::UserMethod { func: m.func, kind: m.kind },
                        None if name == "__exc_init__" => CallClass::UserMethod { func: usize::MAX, kind: MethodKind::Instance },
                        None => CallClass::Bridge,
                    };
                }
            }
        }
        if !matches!(func.kind, ExprKind::GetAttr { .. } | ExprKind::PyBuiltin(_) | ExprKind::Slot(_)) {
            match self.expr(func) {
                Some(t) if matches!(t.as_comp(), Some(Comp::Type(..))) => {
                    let Some(Comp::Type(c, _)) = t.as_comp() else { unreachable!() };
                    return CallClass::Construct(c);
                }
                Some(t) if t.as_closure().is_some() || t.union_members().is_some_and(|ms| ms.iter().all(|m| m.as_closure().is_some())) => return CallClass::ClosureCall,
                None => return CallClass::Pending,
                _ => {}
            }
        }
        match &func.kind {
            ExprKind::GetAttr { obj, name } => match self.expr(obj) {
                Some(t) if ops::base(t).as_record().is_some() => {
                    let fs = ops::base(t).as_record().unwrap();
                    let _ = &fs;
                    if name == "get" && (1..=2).contains(&args.len()) && kwargs.is_empty() {
                        if let ExprKind::Str(k) = &args[0].kind {
                            return CallClass::RecordGet(pyfast_ir::types::intern_str(k));
                        }
                    }
                    // Any other method: the record is a plain dict here.
                    let d = self.degrade(obj, ops::base(t));
                    let argtys: Option<Vec<Ty>> = args.iter().map(|a| self.expr(a)).collect();
                    let kwtys: Option<Vec<(String, Ty)>> = kwargs.iter().map(|(k, v)| self.expr(v).map(|t| (k.clone(), t))).collect();
                    match (argtys, kwtys) {
                        (Some(a), Some(k)) if native::method(d, name, &a, &k, self.boxing()).is_some() => CallClass::Method { recv: d, name: name.clone() },
                        (Some(_), Some(_)) => CallClass::Bridge,
                        _ => CallClass::Pending,
                    }
                }
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
                let a: Vec<Ty> = a.iter().zip(args).map(|(t, x)| self.record_arg(name, x, *t)).collect();
                if name == "isinstance" && a.len() == 2 {
                    // Native classes: decided by the class hierarchy.
                    let natives: Option<Vec<ClassId>> = match &args[1].kind {
                        ExprKind::ClassRef(c) => Some(vec![*c]),
                        ExprKind::Tuple(items) if items.iter().all(|i| matches!(i.kind, ExprKind::ClassRef(_))) => {
                            Some(items.iter().map(|i| if let ExprKind::ClassRef(c) = i.kind { c } else { unreachable!() }).collect())
                        }
                        _ => None,
                    };
                    if let Some(cs) = natives {
                        if a[0] == Ty::Obj {
                            return CallClass::Bridge;
                        }
                        if let Some(ms) = a[0].union_members() {
                            let hits: Vec<bool> = ms.iter().map(|m| cs.iter().any(|k| self.prog.module.is_subclass(m.class_id().unwrap(), *k))).collect();
                            if hits.iter().all(|h| *h) || hits.iter().all(|h| !*h) {
                                return CallClass::StaticBool(hits[0]);
                            }
                            return CallClass::RuntimeIsinstance(cs);
                        }
                        let v = a[0].class_id().is_some_and(|c| cs.iter().any(|k| self.prog.module.is_subclass(c, *k)));
                        return CallClass::StaticBool(v);
                    }
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

    /// One comparison where an operand is a native instance.
    fn compare_classes(&mut self, e: &Expr, op: CmpOp, a: Ty, ae: &Expr, b: Ty, be: &Expr) {
        match op {
            CmpOp::Is | CmpOp::IsNot => {}
            CmpOp::In | CmpOp::NotIn => {
                if class_like(b) {
                    if self.dunder(e, b, "__contains__", &[ae]).is_none() {
                        self.report(Diagnostic::error("P301", e.span, format!("argument of type '{b}' is not iterable")));
                    }
                } else if !ops::compare(op, a, b) {
                    self.report(Diagnostic::error("P301", e.span, format!("'in <{b}>' with a {a} operand")));
                }
            }
            _ => {
                let (fwd, refl) = cmp_dunder(op).unwrap();
                if self.dunder(e, a, fwd, &[be]).is_some() || self.dunder(e, b, refl, &[ae]).is_some() {
                    return;
                }
                if op == CmpOp::NotEq && self.dunder(e, a, "__eq__", &[be]).is_some() {
                    return;
                }
                if !matches!(op, CmpOp::Eq | CmpOp::NotEq) {
                    self.report(
                        Diagnostic::error("P301", e.span, format!("'{}' not supported between instances of '{a}' and '{b}'", match op {
                            CmpOp::Lt => "<",
                            CmpOp::LtE => "<=",
                            CmpOp::Gt => ">",
                            _ => ">=",
                        }))
                        .note("define __lt__ (and friends) on the class"),
                    );
                }
            }
        }
    }

    /// Call a function value with argument expressions (bound to its signature).
    fn call_value_exprs(&mut self, e: &Expr, ft: Ty, args: &[Expr], kwargs: &[(String, Expr)]) -> Option<Ty> {
        if let Some(ms) = ft.union_members() {
            let mut out: Option<Ty> = None;
            for m in ms {
                let r = self.call_value_exprs(e, m, args, kwargs)?;
                out = Some(match out {
                    None => r,
                    Some(p) => native::ty_join(p, r, self.boxing()).or(self.boxing().then_some(Ty::Obj))?,
                });
            }
            return out;
        }
        let (fid, _) = ft.as_closure()?;
        let bound = self.bind(e, fid, false, args, kwargs)?;
        let ExprKind::CallObj { func, .. } = &e.kind else { return None };
        let bound = env_defaults(bound, func);
        let lead: Vec<Ty> = if env_offset(&self.prog.module.funcs[fid]) == 1 { vec![ft] } else { vec![] };
        self.call_spec(e, fid, &lead, &bound)
    }

    /// Call a function value with positional argument types (key functions, map, filter).
    fn call_value_tys(&mut self, e: &Expr, ft: Ty, arg_tys: &[Ty]) -> Option<Ty> {
        let (fid, _) = ft.as_closure()?;
        let f = &self.prog.module.funcs[fid];
        if f.sig.n_pos < arg_tys.len() || f.sig.names.len() > arg_tys.len() && f.sig.defaults.iter().skip(arg_tys.len()).any(|d| d.is_none()) {
            self.report(Diagnostic::error("P303", e.span, format!("{}() is called with {} argument(s) here", f.name, arg_tys.len())));
            return None;
        }
        let mut args: Vec<Ty> = if env_offset(f) == 1 { vec![ft] } else { vec![] };
        args.extend(arg_tys);
        // Defaults fill the remaining parameters (closure-stored ones from the value's type).
        let (_, caps) = ft.as_closure()?;
        let defaults: Vec<Expr> = f.sig.defaults.iter().skip(arg_tys.len()).flatten().cloned().collect();
        for d in &defaults {
            match d.kind {
                ExprKind::EnvDefault(i) => args.push(*caps.get(i)?),
                _ => args.push(self.expr(d)?),
            }
        }
        let key = SpecKey { func: fid, args };
        let r = self.prog.spec(&key).and_then(|s| self.prog.call_result(s));
        if let Some(req) = self.requests.as_deref_mut() {
            req.push((key, e.span));
        }
        r
    }

    /// `sorted(xs, key=f)`, `max(xs, key=f)`, `map(f, xs)`, `filter(f, xs)` with a native function value.
    fn uses_fn_value(&mut self, name: &str, args: &[Expr], kwargs: &[(String, Expr)]) -> bool {
        let is_fn = |me: &mut Self, e: &Expr| me.expr(e).is_some_and(|t| t.as_closure().is_some());
        match name {
            "sorted" | "min" | "max" => kwargs.iter().any(|(k, v)| k == "key" && is_fn(self, v)),
            "map" | "filter" => args.first().is_some_and(|f| is_fn(self, f)),
            _ => false,
        }
    }

    fn higher_order(&mut self, e: &Expr, name: &str, args: &[Expr], kwargs: &[(String, Expr)]) -> Option<Ty> {
        match name {
            "sorted" | "min" | "max" => {
                let xs = self.expr(args.first()?)?;
                let el = native::iter_elem(xs)?;
                let kf = kwargs.iter().find(|(k, _)| k == "key").map(|(_, v)| v)?;
                let ft = self.expr(kf)?;
                let kt = if el.is_never() { Some(Ty::Int) } else { self.call_value_tys(e, ft, &[el]) };
                kt?;
                for (k, v) in kwargs {
                    if k != "key" {
                        self.expr(v)?;
                    }
                }
                Some(if name == "sorted" { Ty::list(el) } else { el })
            }
            "map" => {
                let ft = self.expr(&args[0])?;
                let els: Vec<Ty> = args[1..].iter().map(|a| self.expr(a).and_then(native::iter_elem)).collect::<Option<_>>()?;
                let r = self.call_value_tys(e, ft, &els)?;
                Some(Ty::list(r))
            }
            _ => {
                let ft = self.expr(&args[0])?;
                let el = native::iter_elem(self.expr(&args[1])?)?;
                self.call_value_tys(e, ft, &[el])?;
                Some(Ty::list(el))
            }
        }
    }

    fn box_class(&mut self, e: &Expr, c: ClassId) {
        if self.prog.boxed_classes.contains(&c) && self.diags.is_none() {
            // Already boxed: still request (idempotent), cheap.
        }
        let t = self.class_ty(c);
        self.news.push((c, Some(t)));
        let m = self.prog.module;
        let mut seen: Vec<String> = Vec::new();
        for k in m.mro(c) {
            for meth in &m.class(k).methods {
                if seen.contains(&meth.name) || meth.kind == MethodKind::Setter {
                    continue;
                }
                seen.push(meth.name.clone());
                let f = &m.funcs[meth.func];
                let n_params = f.sig.names.len();
                let args: Vec<Ty> = match meth.kind {
                    MethodKind::Static => (0..n_params).map(|_| Ty::Obj).collect(),
                    MethodKind::Class => std::iter::once(Ty::class_type(c, &m.class(c).rust)).chain((1..n_params).map(|_| Ty::Obj)).collect(),
                    MethodKind::Property => vec![t],
                    _ => std::iter::once(t).chain((1..n_params).map(|_| Ty::Obj)).collect(),
                };
                if f.sig.vararg.is_some() || f.sig.kwarg.is_some() {
                    continue;
                }
                if let Some(req) = self.requests.as_deref_mut() {
                    req.push((SpecKey { func: meth.func, args }, e.span));
                }
            }
        }
        let fields: Vec<(String, Ty)> = self.prog.fields.iter().filter(|((k, _), _)| *k == c).filter_map(|((_, n), s)| s.known().map(|t| (n.clone(), t))).collect();
        for (n, ft) in fields {
            if ft.has_mutable() && ft != Ty::Obj {
                self.fassigns.push(((c, n), Ty::Obj));
            }
        }
    }

    /// Type-check a Rust/C call's arguments; callbacks get specializations
    /// for the Rust parameter types. Returns the result type.
    fn extern_args(&mut self, e: &Expr, ext: usize, func: usize, args: &[Expr]) -> Option<Ty> {
        let tys: Vec<Option<Ty>> = args.iter().map(|a| self.expr(a)).collect();
        let tys: Vec<Ty> = tys.into_iter().collect::<Option<_>>()?;
        let m = &self.prog.module.externs[ext];
        let f = &m.funcs[func];
        if f.params.len() != tys.len() {
            self.report(Diagnostic::error("P303", e.span, format!("{}.{}() takes {} argument(s) but {} were given", m.name, f.name.replace("::", "."), f.params.len(), tys.len())));
            return None;
        }
        let params = f.params.clone();
        let (mname, fname) = (m.name.clone(), f.name.replace("::", "."));
        for (i, (p, t)) in params.iter().zip(&tys).enumerate() {
            if !p.accepts(*t) {
                self.report(
                    Diagnostic::error("P303", args[i].span, format!("argument {} of {mname}.{fname}() expects {} (`{}`), got {t}", i + 1, p.ty(), p.rust()))
                        .note("CPython-side code would raise TypeError here"),
                );
                continue;
            }
            if let ExtTy::Callback(ps, _, _) | ExtTy::CFn(ps, _) = p {
                let pts: Vec<Ty> = ps.iter().map(|x| x.ty()).collect();
                for member in t.union_members().unwrap_or(vec![*t]) {
                    self.call_value_tys(e, member, &pts);
                }
            }
        }
        Some(self.prog.module.externs[ext].funcs[func].ret.ty())
    }

    fn class_ty(&self, c: ClassId) -> Ty {
        Ty::class(c, &self.prog.module.class(c).rust)
    }

    /// Bind call arguments (with the receiver slot removed for methods).
    fn bind(&mut self, e: &Expr, func: FuncId, skip_first: bool, args: &[Expr], kwargs: &[(String, Expr)]) -> Option<Vec<Expr>> {
        let (args, kwargs) = {
            let mut ty = |x: &Expr| self.expr(x);
            expand_stars(&mut ty, args, kwargs)
        };
        let (args, kwargs) = (&args[..], &kwargs[..]);
        let f = &self.prog.module.funcs[func];
        let mut sig = f.sig.clone();
        if skip_first && !sig.names.is_empty() {
            sig.names.remove(0);
            sig.n_pos = sig.n_pos.saturating_sub(1);
            sig.n_posonly = sig.n_posonly.saturating_sub(1);
            if !sig.defaults.is_empty() {
                sig.defaults.remove(0);
            }
        }
        match bind_call(&sig, &f.name, args.to_vec(), kwargs.to_vec(), e.span) {
            Ok((a, _)) => Some(a),
            Err(BindError::TypeError(m)) | Err(BindError::Unsupported(m)) => {
                self.report(Diagnostic::error("P303", e.span, m).note("CPython would raise TypeError here"));
                None
            }
        }
    }

    /// Call specialization `func` with leading receiver types and bound arguments.
    fn call_spec(&mut self, e: &Expr, func: FuncId, lead: &[Ty], bound: &[Expr]) -> Option<Ty> {
        let tys: Vec<Option<Ty>> = bound.iter().map(|a| self.expr(a)).collect();
        let tys: Vec<Ty> = tys.into_iter().collect::<Option<_>>()?;
        let mut all = lead.to_vec();
        all.extend(&tys);
        let key = SpecKey { func, args: all };
        let callee = self.prog.spec(&key);
        let r = callee.and_then(|s| self.prog.call_result(s));
        if let Some(s) = callee.filter(|s| s.analyzed) {
            let f = &self.prog.module.funcs[func];
            for (i, (a, t)) in bound.iter().zip(&tys).enumerate() {
                if let Some(pt) = f.params.get(lead.len() + i).and_then(|p| s.locals.get(p)).and_then(|x| x.known()) {
                    self.flow(a, *t, pt);
                }
            }
        }
        if let Some(req) = self.requests.as_deref_mut() {
            req.push((key, e.span));
        }
        r
    }

    fn method_call(&mut self, e: &Expr, recv: Ty, _name: &str, fid: FuncId, kind: MethodKind, args: &[Expr], kwargs: &[(String, Expr)]) -> Option<Ty> {
        let class = recv.class_id().or(match recv.as_comp() {
            Some(Comp::Type(c, _)) => Some(c),
            _ => None,
        })?;
        match kind {
            MethodKind::Static => {
                let bound = self.bind(e, fid, false, args, kwargs)?;
                self.call_spec(e, fid, &[], &bound)
            }
            MethodKind::Class => {
                let bound = self.bind(e, fid, true, args, kwargs)?;
                let t = Ty::class_type(class, &self.prog.module.class(class).rust);
                self.call_spec(e, fid, &[t], &bound)
            }
            _ => {
                if recv.class_id().is_none() {
                    self.report(Diagnostic::error("P303", e.span, "instance method called on the class without an instance"));
                    return None;
                }
                let bound = self.bind(e, fid, true, args, kwargs)?;
                self.call_spec(e, fid, &[recv], &bound)
            }
        }
    }

    /// Dunder method `name` on a native instance: `Some(result)` when defined.
    fn dunder(&mut self, e: &Expr, recv: Ty, name: &str, others: &[&Expr]) -> Option<Option<Ty>> {
        if let Some(ms) = recv.union_members() {
            if !ms.iter().all(|m| self.prog.module.find_method(m.class_id().unwrap(), name).is_some()) {
                return None;
            }
            let mut out: Option<Ty> = None;
            for m in ms {
                let r = self.dunder(e, m, name, others)?;
                let Some(r) = r else { return Some(None) };
                out = Some(match out {
                    None => r,
                    Some(prev) => match native::ty_join(prev, r, self.boxing()) {
                        Some(j) => j,
                        None => return Some(None),
                    },
                });
            }
            return Some(out);
        }
        let c = recv.class_id()?;
        let m = self.prog.module.find_method(c, name)?.clone();
        let bound: Vec<Expr> = others.iter().map(|x| (*x).clone()).collect();
        Some(self.call_spec(e, m.func, &[recv], &bound))
    }

    fn new_instance(&mut self, e: &Expr, c: ClassId, args: &[Expr], kwargs: &[(String, Expr)]) -> Option<Ty> {
        let t = self.class_ty(c);
        self.news.push((c, None));
        // The trait impls (printing, ==, <, truthiness, hashing) call these.
        let unary = ["__repr__", "__str__", "__len__", "__bool__", "__hash__"];
        for d in unary {
            if let Some(m) = self.prog.module.find_method(c, d) {
                if let Some(req) = self.requests.as_deref_mut() {
                    req.push((SpecKey { func: m.func, args: vec![t] }, e.span));
                }
            }
        }
        for d in ["__eq__", "__lt__"] {
            if let Some(m) = self.prog.module.find_method(c, d) {
                if let Some(req) = self.requests.as_deref_mut() {
                    req.push((SpecKey { func: m.func, args: vec![t, t] }, e.span));
                }
            }
        }
        match self.prog.module.find_method(c, "__init__").cloned() {
            Some(m) => {
                let bound = self.bind(e, m.func, true, args, kwargs)?;
                self.call_spec(e, m.func, &[t], &bound)?;
            }
            None => {
                if !args.is_empty() || !kwargs.is_empty() {
                    if self.prog.module.exception_base(c).is_some() {
                        for a in args {
                            self.expr(a)?;
                        }
                    } else {
                        self.report(Diagnostic::error("P303", e.span, format!("{}() takes no arguments", self.prog.module.class(c).name)).note("CPython would raise TypeError here"));
                        return None;
                    }
                }
            }
        }
        Some(t)
    }

    /// `obj.name` on a native instance (or a class object).
    fn attr(&mut self, e: &Expr, recv: Ty, name: &str) -> Option<Ty> {
        if let Some(ms) = recv.union_members() {
            let mut out: Option<Ty> = None;
            for mt in ms {
                let r = self.attr(e, mt, name)?;
                out = Some(match out {
                    None => r,
                    Some(prev) => native::ty_join(prev, r, self.boxing()).or(self.boxing().then_some(Ty::Obj))?,
                });
            }
            return out;
        }
        let m = self.prog.module;
        if let Some(Comp::Type(c, _)) = recv.as_comp() {
            return match m.find_class_var(c, name) {
                Some(owner) => {
                    let cd = m.class(owner);
                    let short = cd.name.rsplit('.').next().unwrap_or("").to_string();
                    self.prog.globals.get(&(cd.unit, format!("{short}.{name}"))).and_then(|s| s.known())
                }
                None => {
                    self.report(Diagnostic::error("P301", e.span, format!("type object '{}' has no attribute '{name}'", m.class(c).name)));
                    None
                }
            };
        }
        let c = recv.class_id()?;
        if let Some(meth) = m.find_method(c, name).cloned() {
            return match meth.kind {
                MethodKind::Property => self.call_spec(e, meth.func, &[recv], &[]),
                _ => {
                    self.report(
                        Diagnostic::error("P301", e.span, format!("method `{name}` of `{}` used as a value", m.class(c).name))
                            .note("call it directly; bound methods as values are phase 4"),
                    );
                    None
                }
            };
        }
        if name == "__class__" || name == "__dict__" {
            self.report(Diagnostic::error("P301", e.span, format!("`{name}` of a native instance is not supported yet")));
            return None;
        }
        let field = self.prog.fields.get(&(c, name.to_string())).and_then(|s| s.known());
        let cvar = m.find_class_var(c, name).and_then(|owner| {
            let cd = m.class(owner);
            let short = cd.name.rsplit('.').next().unwrap_or("").to_string();
            self.prog.globals.get(&(cd.unit, format!("{short}.{name}"))).and_then(|s| s.known())
        });
        match (field, cvar) {
            (Some(f), Some(v)) => native::ty_join(f, v, self.boxing()).or(self.boxing().then_some(Ty::Obj)),
            (Some(f), None) => Some(f),
            (None, Some(v)) => Some(v),
            (None, None) => {
                let declared = m.declared_fields(c).iter().any(|f| f == name) || m.find_class_var(c, name).is_some();
                if declared {
                    return None; // assigned somewhere; type not known yet
                }
                // Never assigned anywhere: reading it always raises AttributeError.
                self.report(
                    Diagnostic::warning("P308", e.span, format!("'{}' object has no attribute '{name}'", m.class(c).name))
                        .note("no assignment to this attribute exists; it raises AttributeError at runtime"),
                );
                Some(Ty::never())
            }
        }
    }

    fn expr_shallow(&mut self, e: &Expr) -> Option<Ty> {
        self.expr(e)
    }

    pub fn expr(&mut self, e: &Expr) -> Option<Ty> {
        match &e.kind {
            ExprKind::New { class, args, kwargs } => self.new_instance(e, *class, args, kwargs),
            ExprKind::ClassRef(c) => Some(Ty::class_type(*c, &self.prog.module.class(*c).rust)),
            ExprKind::EnvDefault(_) => None,
            ExprKind::EnvRead { value, index } => {
                let (_, caps) = self.expr(value)?.as_closure()?;
                caps.get(*index).copied()
            }
            ExprKind::MakeClosure { func, captures, defaults } => {
                let mut caps: Vec<Ty> = captures.iter().map(|n| self.local_ty(n)).collect::<Option<_>>()?;
                for d in defaults {
                    caps.push(self.expr(d)?);
                }
                let t = Ty::closure(*func as u32, caps.clone(), &self.prog.module.funcs[*func].name);
                // `nonlocal` writes in the closure widen the shared variables here.
                for s in self.prog.specs.iter().filter(|s| s.key.func == *func && s.analyzed && s.key.args.first() == Some(&t)) {
                    for (n, ct) in captures.iter().zip(&caps) {
                        if let Some(nt) = s.locals.get(n).and_then(|x| x.known()) {
                            if nt != *ct {
                                self.widens.push((n.clone(), nt));
                            }
                        }
                    }
                }
                Some(t)
            }
            ExprKind::Yield(v) => {
                let t = match v {
                    Some(e) => {
                        let t = self.expr(e)?;
                        if let Some(dest) = self.cur_ret {
                            self.flow(e, t, dest);
                        }
                        t
                    }
                    None => Ty::None,
                };
                self.yields.push(t);
                Some(self.cur_send)
            }
            ExprKind::YieldFrom(v) => {
                let t = self.expr(v)?;
                if let Some((y, s, r)) = t.as_gen() {
                    self.yields.push(y);
                    // Values sent to this generator are forwarded to `v`.
                    if self.cur_send != Ty::None && native::ty_join(s, self.cur_send, self.boxing()) != Some(s) {
                        let joined = native::ty_join(s, self.cur_send, self.boxing()).unwrap_or(Ty::Obj);
                        self.widen(v, Ty::gen3(y, joined, r));
                    }
                    return Some(r);
                }
                let el = native::iter_elem(t)?;
                self.yields.push(el);
                Some(if t == Ty::Obj { Ty::Obj } else { Ty::None })
            }
            // `math.pi`, `from math import tau`: native float constants.
            ExprKind::GetAttr { .. } | ExprKind::Slot(_) if self.prog.math_const_of(e).is_some() => Some(Ty::Float),
            ExprKind::Await(v) => {
                let t = self.expr(v)?;
                if let Some(r) = t.as_coro() {
                    return Some(r);
                }
                if t == Ty::Obj {
                    return Some(Ty::Obj);
                }
                self.report(Diagnostic::error("P301", v.span, format!("object {t} can't be used in 'await' expression")).note("CPython would raise TypeError here"));
                None
            }
            ExprKind::Int(_) => Some(Ty::Int),
            ExprKind::Float(_) => Some(Ty::Float),
            ExprKind::Str(_) => Some(Ty::Str),
            ExprKind::Bool(_) => Some(Ty::Bool),
            ExprKind::NoneLit => Some(Ty::None),
            ExprKind::Local(n) => self.local_read(n, e.span),
            ExprKind::Global { unit, name } => self.prog.globals.get(&(*unit, name.clone())).and_then(|s| s.known()),
            ExprKind::BinOp { op, left, right } => {
                let l = self.expr(left);
                let r = self.expr(right);
                let (l, r) = (l?, r?);
                if class_like(l) || class_like(r) {
                    let n = dunder_name(*op);
                    if let Some(t) = self.dunder(e, l, &format!("__{n}__"), &[right]) {
                        return t;
                    }
                    if let Some(t) = self.dunder(e, r, &format!("__r{n}__"), &[left]) {
                        return t;
                    }
                    self.report(Diagnostic::error("P301", e.span, format!("unsupported operand type(s) for {}: '{l}' and '{r}'", op.symbol())).note("define the dunder method on the class"));
                    return None;
                }
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
                let coll = |t: Ty| t.as_comp().is_some_and(|c| !matches!(c, Comp::Opt(_) | Comp::Num));
                if coll(l) || coll(r) {
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
                if class_like(t) && *op != UnaryOp::Not {
                    let n = match op {
                        UnaryOp::Neg => "__neg__",
                        UnaryOp::Pos => "__pos__",
                        _ => "__invert__",
                    };
                    return match self.dunder(e, t, n, &[]) {
                        Some(r) => r,
                        None => {
                            self.report(Diagnostic::error("P301", e.span, format!("bad operand type for unary operator: '{t}'")));
                            None
                        }
                    };
                }
                if class_like(t) {
                    // `not obj` uses __bool__/__len__ (requested at construction).
                    return Some(Ty::Bool);
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
                        if class_like(a) || class_like(b) {
                            self.compare_classes(c, *op, a, prev, b, c);
                        } else if a == Ty::Obj || b == Ty::Obj {
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
                // `x is not None and x.f()`: later operands run only when the
                // earlier ones were true (`and`) or false (`or`).
                let start = self.nn.clone();
                let mut tys: Vec<Option<Ty>> = Vec::new();
                for v in values {
                    tys.push(self.expr(v));
                    let (tf, ff) = test_facts(v);
                    self.nn_add(if *op == BoolOpKind::And { &tf } else { &ff });
                }
                self.nn = start;
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
                let (tf, ff) = test_facts(test);
                let start = self.nn.clone();
                self.nn_add(&tf);
                let a = self.expr(body);
                self.nn = start.clone();
                self.nn_add(&ff);
                let b = self.expr(orelse);
                self.nn = start;
                self.merge(&[a?, b?], e, "conditional expression")
            }
            ExprKind::ExternCall { ext, func, args } => self.extern_args(e, *ext, *func, args),
            ExprKind::Walrus { target, global, value } => {
                let t = self.expr(value);
                match global {
                    Some(u) => {
                        if let Some(t) = t {
                            self.gassigns.push(((*u, target.clone()), t));
                        }
                    }
                    None => {
                        if let Some(t) = t {
                            if self.local_ty(target) != Some(t) {
                                self.widens.push((target.clone(), t));
                            }
                        }
                        self.nn_assign(target, t);
                    }
                }
                t
            }
            ExprKind::Annotated { ty, value } if ty.as_comp().is_some() => {
                // A container annotation on a native value: its element types
                // join the annotation's (`parts: list[str] = []`).
                let v = self.expr(value)?;
                match native::ty_join(v, *ty, false) {
                    Some(j) if v != Ty::Obj && j != Ty::Obj => Some(j),
                    _ => Some(v),
                }
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
            ExprKind::GetAttr { obj, name } if name == "__name__" && type_call_arg(obj).is_some() => {
                // `type(x).__name__` of a native value is known statically.
                let t = self.expr(type_call_arg(obj).unwrap())?;
                if t == Ty::Obj {
                    self.expr(obj);
                    return Some(Ty::Obj);
                }
                Some(Ty::Str)
            }
            ExprKind::GetAttr { obj, name } => {
                let t = self.expr(obj)?;
                if t == Ty::Obj {
                    return Some(Ty::Obj);
                }
                // A value that cannot exist (an element of a list that never
                // receives one, in an early round): so is its attribute.
                if t.is_never() {
                    return Some(Ty::never());
                }
                if let Some((ext, si)) = t.as_ext_struct() {
                    let st = &self.prog.module.externs[ext].structs[si];
                    return match st.fields.iter().find(|(n, _)| n == name) {
                        Some((_, ft)) => Some(ft.ty()),
                        None => {
                            self.report(Diagnostic::error("P301", e.span, format!("'{}' has no public field '{name}'", st.name)));
                            None
                        }
                    };
                }
                if class_like(t) || matches!(t.as_comp(), Some(Comp::Type(..))) {
                    return self.attr(e, t, name);
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
                // Distinct literal string keys: a record (one type per key).
                let skeys: Option<Vec<&str>> = pairs.iter().map(|(k, _)| if let ExprKind::Str(s) = &k.kind { Some(s.as_str()) } else { None }).collect();
                if let Some(ks) = skeys.filter(|ks| !ks.is_empty() && ks.iter().collect::<HashSet<_>>().len() == ks.len()) {
                    let vts: Vec<Ty> = pairs.iter().map(|(_, v)| self.expr(v)).collect::<Option<_>>()?;
                    if vts.iter().all(|t| native::record_field_ok(*t)) {
                        return Some(Ty::record(ks.iter().zip(vts).map(|(k, t)| Field { key: pyfast_ir::types::intern_str(k), ty: t, optional: false }).collect()));
                    }
                }
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
                if class_like(ot) {
                    return match self.dunder(e, ot, "__getitem__", &[index]) {
                        Some(t) => t,
                        None => {
                            self.report(Diagnostic::error("P301", e.span, format!("'{ot}' object is not subscriptable")));
                            None
                        }
                    };
                }
                if ot == Ty::Obj {
                    self.index_types(index);
                    self.escape(index);
                    return Some(Ty::Obj);
                }
                // `record["key"]`: the field's type (a missing key raises KeyError).
                if let Some(fs) = ops::base(ot).as_record() {
                    if let ExprKind::Str(k) = &index.kind {
                        return Some(fs.iter().find(|f| f.key == k.as_str()).map(|f| f.ty).unwrap_or(Ty::never()));
                    }
                }
                let ot = if ops::base(ot).as_record().is_some() { self.degrade(obj, ops::base(ot)) } else { ot };
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
                    Callee::Builtin(b) if tys.first().is_some_and(|t| class_like(*t)) && matches!(b, Builtin::Len | Builtin::Abs | Builtin::Int | Builtin::Float) => {
                        let n = match b {
                            Builtin::Len => "__len__",
                            Builtin::Abs => "__abs__",
                            Builtin::Int => "__int__",
                            _ => "__float__",
                        };
                        match self.dunder(e, tys[0], n, &[]) {
                            Some(t) => t,
                            None => {
                                self.report(Diagnostic::error("P303", e.span, format!("object of type '{}' has no {n}", tys[0])).note("CPython would raise TypeError here"));
                                None
                            }
                        }
                    }
                    Callee::Builtin(b @ (Builtin::Str | Builtin::Bool | Builtin::Print | Builtin::PrintKw)) if tys.iter().any(|t| class_like(*t)) => {
                        Some(if matches!(b, Builtin::Str) { Ty::Str } else if matches!(b, Builtin::Bool) { Ty::Bool } else { Ty::None })
                    }
                    // An argument that cannot exist: the call is never reached.
                    Callee::Builtin(b) if !matches!(b, Builtin::Print | Builtin::PrintKw) && tys.iter().any(|t| t.is_never()) => Some(Ty::never()),
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
                        let key = self.prog.resolve_key(SpecKey { func: *fid, args: tys.clone() });
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
        // A method call on a value that cannot exist (`for s in xs` over a
        // list that is still empty in this round) cannot return: Never,
        // not a Python object (joins never narrow, so `object` would stick).
        if let ExprKind::GetAttr { obj, .. } = &func.kind {
            if matches!(obj.kind, ExprKind::Local(_)) && self.expr(obj).is_some_and(|t| t.is_never()) {
                for a in args {
                    self.expr(a);
                }
                for (_, v) in kwargs {
                    self.expr(v);
                }
                return Some(Ty::never());
            }
        }
        match self.classify(func, args, kwargs) {
            CallClass::UserMethod { func: fid, kind } => {
                let ExprKind::GetAttr { obj, name } = &func.kind else { unreachable!() };
                let recv = self.expr(obj)?;
                if fid == usize::MAX {
                    // BaseException.__init__(self, msg) from super().__init__ in a user exception.
                    for a in args {
                        self.expr(a)?;
                    }
                    return Some(Ty::None);
                }
                self.method_call(e, recv, name, fid, kind, args, kwargs)
            }
            CallClass::Construct(c) => self.new_instance(e, c, args, kwargs),
            CallClass::ExtMethod { ext, func: fi } => {
                if !kwargs.is_empty() {
                    self.report(Diagnostic::error("P303", e.span, "keyword arguments to Rust/C methods are not supported"));
                    return None;
                }
                let ExprKind::GetAttr { obj, .. } = &func.kind else { unreachable!() };
                self.expr(obj)?;
                self.extern_args(e, ext, fi, args)
            }
            CallClass::ClosureCall => {
                let ft = self.expr(func)?;
                self.call_value_exprs(e, ft, args, kwargs)
            }
            CallClass::RuntimeIsinstance(_) => {
                self.expr(&args[0])?;
                Some(Ty::Bool)
            }
            CallClass::UnionMethod(name) => {
                let ExprKind::GetAttr { obj, .. } = &func.kind else { unreachable!() };
                let recv = self.expr(obj)?;
                let mut out: Option<Ty> = None;
                for m in recv.union_members().unwrap() {
                    let c = m.class_id().unwrap();
                    let Some(meth) = self.prog.module.find_method(c, &name).cloned() else {
                        self.report(Diagnostic::error("P301", e.span, format!("'{}' object has no attribute '{name}'", self.prog.module.class(c).name)).note("every class of the union needs the method"));
                        return None;
                    };
                    let r = self.method_call(e, m, &name, meth.func, meth.kind, args, kwargs)?;
                    out = Some(match out {
                        None => r,
                        Some(prev) => native::ty_join(prev, r, self.boxing()).or(self.boxing().then_some(Ty::Obj))?,
                    });
                }
                out
            }
            CallClass::Pending => {
                // Evaluate the parts (requests, flows) without deciding yet.
                for a in args.iter().chain(kwargs.iter().map(|(_, v)| v)) {
                    self.expr(a);
                }
                None
            }
            CallClass::StaticBool(_) => Some(Ty::Bool),
            CallClass::Math(name) => {
                let a: Vec<Ty> = args.iter().map(|x| self.expr(x).map(ops::base)).collect::<Option<_>>()?;
                let k: Vec<(String, Ty)> = kwargs.iter().map(|(n, v)| self.expr(v).map(|t| (n.clone(), ops::base(t)))).collect::<Option<_>>()?;
                native::math_call(name, &a, &k)?.ok()
            }
            CallClass::RecordGet(key) => {
                let ExprKind::GetAttr { obj, .. } = &func.kind else { unreachable!() };
                let fs = ops::base(self.expr(obj)?).as_record()?;
                let dflt = match args.get(1) {
                    Some(a) => self.expr(a)?,
                    None => Ty::None,
                };
                match fs.iter().find(|f| f.key == key) {
                    // Always present: the default is unused (still evaluated).
                    Some(f) if !f.optional => Some(f.ty),
                    Some(f) => native::ty_join(f.ty, dflt, self.boxing()).or(self.boxing().then_some(Ty::Obj)),
                    None => Some(dflt),
                }
            }
            CallClass::Method { recv, name } => {
                let ExprKind::GetAttr { obj, .. } = &func.kind else { unreachable!() };
                let a: Vec<Ty> = args.iter().map(|x| self.expr(x)).collect::<Option<_>>()?;
                let k: Vec<(String, Ty)> = kwargs.iter().map(|(n, v)| self.expr(v).map(|t| (n.clone(), t))).collect::<Option<_>>()?;
                match native::method(recv, &name, &a, &k, self.boxing())? {
                    Ok(m) => {
                        // `xs.sort(key=f)`: the key function is called on elements.
                        if name == "sort" {
                            if let Some((_, kt)) = k.iter().find(|(n, t)| n == "key" && t.as_closure().is_some()) {
                                if let Some(el) = recv.list_elem().filter(|e| !e.is_never()) {
                                    self.call_value_tys(e, *kt, &[el])?;
                                }
                            }
                        }
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
            CallClass::Builtin(name) if self.uses_fn_value(&name, args, kwargs) => self.higher_order(e, &name, args, kwargs),
            CallClass::Builtin(name) => {
                let a: Vec<Ty> = args.iter().map(|x| self.expr(x).map(|t| self.record_arg(&name, x, t))).collect::<Option<_>>()?;
                let k: Vec<(String, Ty)> = kwargs.iter().map(|(n, v)| self.expr(v).map(|t| (n.clone(), t))).collect::<Option<_>>()?;
                if a.iter().any(|t| t.is_never()) {
                    return Some(Ty::never());
                }
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
                        // Containers are copied; function values and instances still
                        // need their Python wrappers.
                        match self.expr(a) {
                            Some(t) if has_class(t) || t.as_closure().is_some() || t.union_members().is_some() => self.escape(a),
                            Some(_) => {}
                            None => ok = false,
                        }
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
    /// Specializations a node calls through native-class dispatch
    /// (constructors, methods, dunders), for the call graph.
    pub fn class_calls(&self, spec: &Spec, e: &Expr) -> Vec<SpecKey> {
        let mut reqs: Requests = Vec::new();
        {
            let mut ctx = self.ctx_for(spec);
            ctx.requests = Some(&mut reqs);
            match &e.kind {
                ExprKind::New { .. } | ExprKind::CallObj { .. } | ExprKind::BinOp { .. } | ExprKind::Unary { .. } | ExprKind::Compare { .. } | ExprKind::Subscript { .. } | ExprKind::GetAttr { .. } | ExprKind::MakeClosure { .. } => {
                    ctx.expr_shallow(e);
                }
                ExprKind::Call { callee: Callee::Builtin(_), .. } => {
                    ctx.expr_shallow(e);
                }
                _ => {}
            }
        }
        reqs.into_iter().map(|(k, _)| k).collect()
    }

    /// Statement-level dunder calls (`obj[i] = v`, `for x in obj`, `obj.p = v`).
    pub fn class_stmt_calls(&self, spec: &Spec, st: &Stmt) -> Vec<SpecKey> {
        let mut reqs: Requests = Vec::new();
        {
            let mut ctx = self.ctx_for(spec);
            ctx.requests = Some(&mut reqs);
            match &st.kind {
                StmtKind::SetItem { obj, index, value } => {
                    if self.type_of(spec, obj).is_some_and(class_like) {
                        ctx.set_item(obj, index, value);
                    }
                }
                StmtKind::ForIter { iter, .. } => {
                    if let Some(t) = self.type_of(spec, iter).filter(|t| class_like(*t)) {
                        ctx.dunder(iter, t, "__iter__", &[]);
                    }
                }
                StmtKind::SetAttr { obj, name, value } => {
                    if let Some(t) = self.type_of(spec, obj).filter(|t| class_like(*t)) {
                        let mut a = Vec::new();
                        let mut r = Slot::Unknown;
                        let _ = (name, value, t);
                        ctx.stmt(st, &mut a, &mut r);
                    }
                }
                _ => {}
            }
        }
        reqs.into_iter().map(|(k, _)| k).collect()
    }

    /// Type of `recv.name` (a native instance attribute) in `spec`.
    pub fn attr_type(&self, spec: &Spec, recv: Ty, name: &str) -> Option<Ty> {
        let mut ctx = self.ctx_for(spec);
        ctx.attr(&Expr { kind: ExprKind::NoneLit, span: Span::default() }, recv, name)
    }

    /// Result type of method `name` called on instance type `recv` with these arguments.
    pub fn method_type(&self, spec: &Spec, recv: Ty, func: FuncId, kind: MethodKind, args: &[Expr], kwargs: &[(String, Expr)]) -> Option<Ty> {
        let mut ctx = self.ctx_for(spec);
        ctx.method_call(&Expr { kind: ExprKind::NoneLit, span: Span::default() }, recv, "", func, kind, args, kwargs)
    }

    /// Arguments of a call of `func` bound to its parameters (the receiver
    /// slot removed when `skip_first`), with their evaluation order.
    pub fn bound_args(&self, spec: &Spec, func: FuncId, skip_first: bool, args: &[Expr], kwargs: &[(String, Expr)], span: Span) -> Option<(Vec<Expr>, Vec<usize>)> {
        let (args, kwargs) = {
            let mut ty = |x: &Expr| self.type_of(spec, x);
            expand_stars(&mut ty, args, kwargs)
        };
        let (args, kwargs) = (&args[..], &kwargs[..]);
        let f = &self.module.funcs[func];
        let mut sig = f.sig.clone();
        if skip_first && !sig.names.is_empty() {
            sig.names.remove(0);
            sig.n_pos = sig.n_pos.saturating_sub(1);
            sig.n_posonly = sig.n_posonly.saturating_sub(1);
            if !sig.defaults.is_empty() {
                sig.defaults.remove(0);
            }
        }
        bind_call(&sig, &f.name, args.to_vec(), kwargs.to_vec(), span).ok()
    }

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
            fassigns: vec![],
            news: vec![],
            generator: self.module.funcs[spec.key.func].is_generator,
            yields: vec![],
            cur_send: gen_send_ty(spec),
            gen_returns: vec![],
            send_widen: vec![],
            nn: None,
            nn_reads: vec![],
            narrowed: Some(&spec.narrowed),
        }
    }
}

/// Whether control can continue after `s` (the rule inference uses to stop
/// analyzing a block; codegen stops emitting at the same point, so statements
/// it never typed are never emitted).
pub fn falls_through(locals: &BTreeMap<String, Slot>, s: &Stmt) -> bool {
    match &s.kind {
        StmtKind::Return(_) | StmtKind::Raise(_) => false,
        StmtKind::If { test, body, orelse } => match static_truth(locals, test) {
            Some(true) => block_falls_through(locals, body),
            Some(false) => block_falls_through(locals, orelse),
            None => block_falls_through(locals, body) || block_falls_through(locals, orelse),
        },
        StmtKind::While { test, body } => !(matches!(test.kind, ExprKind::Bool(true)) && !contains_break(body)),
        _ => true,
    }
}

pub fn block_falls_through(locals: &BTreeMap<String, Slot>, stmts: &[Stmt]) -> bool {
    stmts.iter().all(|s| falls_through(locals, s))
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

/// Builtins that take a record as it is (iterating it yields its keys).
pub const RECORD_BUILTINS: &[&str] = &["len", "str", "repr", "bool", "sorted", "list", "tuple", "set", "min", "max", "any", "all", "enumerate", "zip", "reversed"];

/// The record `fs` with key `k` of type `t` (a new key becomes an optional
/// field). `None` when the value cannot be a record field.
fn record_with(fs: &[Field], k: &str, t: Ty) -> Option<Ty> {
    if !native::record_field_ok(t) {
        return None;
    }
    let mut out = fs.to_vec();
    match out.iter_mut().find(|f| f.key == k) {
        Some(f) => f.ty = t,
        None => out.push(Field { key: pyfast_ir::types::intern_str(k), ty: t, optional: true }),
    }
    Some(Ty::record(out))
}
