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
    };
    let main_key = SpecKey { func: module.main, args: vec![] };
    add_spec(&mut prog, main_key);

    let mut rounds = 0;
    loop {
        rounds += 1;
        let mut changed = false;
        let mut i = 0;
        while i < prog.specs.len() {
            let mut requests = Vec::new();
            let (locals, ret, falls, gassigns, ret_widen, (fassigns, news)) = analyze(&prog, i, &mut requests, None);
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
) -> (BTreeMap<String, Slot>, Slot, bool, Vec<(GlobalKey, Ty)>, Vec<(SpecKey, Ty)>, (Vec<((ClassId, String), Ty)>, Vec<(ClassId, Option<Ty>)>)) {
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
    loop {
        let before = locals.clone();
        ret = spec.ret_floor;
        let mut ctx = Ctx { prog, locals: &locals, requests: Some(requests), diags: None, gassigns: vec![], widens: vec![], ret_widen: vec![], cur_ret, fassigns: vec![], news: vec![] };
        let mut assigns = Vec::new();
        falls = ctx.block(&f.body, &mut assigns, &mut ret);
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
        let mut ctx = Ctx { prog, locals: &locals, requests: None, diags: Some(d), gassigns: vec![], widens: vec![], ret_widen: vec![], cur_ret, fassigns: vec![], news: vec![] };
        ctx.block(&f.body, &mut sink, &mut ret2);
    }
    (locals, ret, falls, gassigns, ret_widen, (fassigns, news_acc))
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
        if vt != dest && vt.has_mutable() && is_place(value) {
            self.widen(value, dest);
        }
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
                let it = match self.expr(iter) {
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
                self.block(body, assigns, ret);
            }
            StmtKind::SetItem { obj, index, value } => self.set_item(obj, index, value),
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
        if let ExprKind::GetAttr { obj, name } = &func.kind {
            if let Some(t) = self.expr(obj) {
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
