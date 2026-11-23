//! Symbolic generic signatures (design section 5), e.g.
//! `add(A, B) -> R where Add(A, B, R)`.
//!
//! Each unannotated parameter gets a type variable. Operators on variables
//! produce relational constraints; calls instantiate the callee's scheme.
//! Constraints whose inputs become concrete are solved through the same
//! operator table that drives specialization. This is used for `explain`;
//! code generation is driven by the concrete specializations.

use crate::ops;
use pyrock_ir::*;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sym {
    Var(u32),
    Con(Ty),
}

#[derive(Clone, Debug)]
pub enum Constraint {
    /// `Op(a, b, r)`: `a op b` is defined and produces `r`.
    Bin(BinOp, Sym, Sym, Sym),
    /// `Neg(a, r)` etc.
    Unary(&'static str, Sym, Sym),
    /// `Lt(a, b)`: the comparison is defined.
    Cmp(&'static str, Sym, Sym),
    /// Two values flowing into one variable or return must agree.
    Same(Sym, Sym),
    /// `range()` argument / int-only position.
    IsInt(Sym),
    /// Unresolved recursive call: `f(args) -> r`.
    Rec(String, Vec<Sym>, Sym),
}

#[derive(Clone, Debug)]
pub struct Scheme {
    pub params: Vec<Sym>,
    pub ret: Sym,
    pub constraints: Vec<Constraint>,
}

pub struct Schemes {
    pub by_func: BTreeMap<FuncId, Scheme>,
}

pub fn infer_schemes(m: &Module) -> Schemes {
    let mut st = State { m, next: 0, done: HashMap::new(), in_progress: vec![] };
    let mut by_func = BTreeMap::new();
    for f in &m.funcs {
        if !f.is_module {
            let s = st.scheme(f.id);
            by_func.insert(f.id, s);
        }
    }
    Schemes { by_func }
}

struct State<'m> {
    m: &'m Module,
    next: u32,
    done: HashMap<FuncId, Scheme>,
    /// Functions being inferred, with their parameter and return variables
    /// (monomorphic recursion, as in Hindley–Milner).
    in_progress: Vec<(FuncId, Vec<Sym>, Sym)>,
}

struct FnCx {
    env: HashMap<String, Sym>,
    cs: Vec<Constraint>,
    subst: HashMap<u32, Sym>,
    rets: Vec<Sym>,
}

impl FnCx {
    fn resolve(&self, mut s: Sym) -> Sym {
        while let Sym::Var(v) = s {
            match self.subst.get(&v) {
                Some(&n) => s = n,
                None => break,
            }
        }
        s
    }

    fn bind(&mut self, a: Sym, b: Sym) {
        let (a, b) = (self.resolve(a), self.resolve(b));
        match (a, b) {
            (Sym::Var(x), _) if a != b => {
                self.subst.insert(x, b);
            }
            (_, Sym::Var(y)) if a != b => {
                self.subst.insert(y, a);
            }
            _ if a != b => self.cs.push(Constraint::Same(a, b)),
            _ => {}
        }
    }

    /// Solve constraints whose inputs are concrete; repeat to a fixed point.
    fn solve(&mut self) {
        loop {
            let mut progress = false;
            let cs = std::mem::take(&mut self.cs);
            let mut keep = Vec::new();
            for c in cs {
                match c {
                    Constraint::Bin(op, a, b, r) => match (self.resolve(a), self.resolve(b)) {
                        (Sym::Con(x), Sym::Con(y)) => {
                            if let Some(t) = ops::binop(op, x, y, None) {
                                self.bind(r, Sym::Con(t));
                                progress = true;
                            } else {
                                keep.push(Constraint::Bin(op, Sym::Con(x), Sym::Con(y), r));
                            }
                        }
                        (a, b) => keep.push(Constraint::Bin(op, a, b, r)),
                    },
                    Constraint::Unary(n, a, r) => match self.resolve(a) {
                        Sym::Con(x) => {
                            let op = if n == "Invert" { UnaryOp::Invert } else { UnaryOp::Neg };
                            if let Some(t) = ops::unary(op, x) {
                                self.bind(r, Sym::Con(t));
                                progress = true;
                            }
                        }
                        a => keep.push(Constraint::Unary(n, a, r)),
                    },
                    Constraint::Cmp(n, a, b) => {
                        let (a, b) = (self.resolve(a), self.resolve(b));
                        if !matches!((a, b), (Sym::Con(_), Sym::Con(_))) {
                            keep.push(Constraint::Cmp(n, a, b));
                        }
                    }
                    Constraint::IsInt(a) => if let Sym::Var(_) = self.resolve(a) {
                        self.bind(a, Sym::Con(Ty::Int));
                        progress = true;
                    },
                    Constraint::Same(a, b) => {
                        let (a, b) = (self.resolve(a), self.resolve(b));
                        if a != b {
                            if matches!(a, Sym::Var(_)) || matches!(b, Sym::Var(_)) {
                                self.bind(a, b);
                                progress = true;
                            } else {
                                keep.push(Constraint::Same(a, b));
                            }
                        }
                    }
                    other => keep.push(other),
                }
            }
            self.cs.extend(keep);
            if !progress {
                break;
            }
        }
    }
}

impl State<'_> {
    fn fresh(&mut self) -> Sym {
        self.next += 1;
        Sym::Var(self.next)
    }

    fn scheme(&mut self, fid: FuncId) -> Scheme {
        if let Some(s) = self.done.get(&fid) {
            return s.clone();
        }
        let f = &self.m.funcs[fid];
        let mut cx = FnCx { env: HashMap::new(), cs: vec![], subst: HashMap::new(), rets: vec![] };
        // Annotated parameters/returns are concrete; the rest are type variables.
        let params: Vec<Sym> = (0..f.params.len())
            .map(|i| match f.param_ann.get(i).copied().flatten() {
                Some(t) => Sym::Con(t),
                None => self.fresh(),
            })
            .collect();
        let ret = match f.ret_ann {
            Some(t) => Sym::Con(t),
            None => self.fresh(),
        };
        self.in_progress.push((fid, params.clone(), ret));
        for (p, s) in f.params.iter().zip(&params) {
            cx.env.insert(p.clone(), *s);
        }
        let falls = self.block(&f.body, &mut cx);
        if falls {
            cx.rets.push(Sym::Con(Ty::None));
        }
        for r in cx.rets.clone() {
            cx.bind(ret, r);
        }
        cx.solve();
        let params = params.into_iter().map(|p| cx.resolve(p)).collect();
        let ret = cx.resolve(ret);
        let mut seen = std::collections::HashSet::new();
        let cs = cx.cs.iter().map(|c| subst_c(&cx, c)).filter(|c| seen.insert(format!("{c:?}"))).collect();
        let s = Scheme { params, ret, constraints: cs };
        self.in_progress.pop();
        self.done.insert(fid, s.clone());
        s
    }

    fn block(&mut self, stmts: &[Stmt], cx: &mut FnCx) -> bool {
        for s in stmts {
            if !self.stmt(s, cx) {
                return false;
            }
        }
        true
    }

    fn assign(&mut self, name: &str, v: Sym, cx: &mut FnCx) {
        match cx.env.get(name).copied() {
            Some(prev) => cx.cs.push(Constraint::Same(prev, v)),
            None => {
                cx.env.insert(name.to_string(), v);
            }
        }
    }

    fn stmt(&mut self, s: &Stmt, cx: &mut FnCx) -> bool {
        match &s.kind {
            StmtKind::Assign { target, value } => {
                let v = self.expr(value, cx);
                self.assign(target, v, cx);
            }
            StmtKind::AugAssign { target, op, value } => {
                let cur = cx.env.get(target).copied().unwrap_or_else(|| self.fresh());
                let v = self.expr(value, cx);
                let r = self.fresh();
                cx.cs.push(Constraint::Bin(*op, cur, v, r));
                self.assign(target, r, cx);
                cx.solve();
            }
            StmtKind::Expr(e) => {
                self.expr(e, cx);
            }
            StmtKind::Return(v) => {
                let r = match v {
                    Some(e) => self.expr(e, cx),
                    None => Sym::Con(Ty::None),
                };
                cx.rets.push(r);
                return false;
            }
            StmtKind::If { test, body, orelse } => {
                self.expr(test, cx);
                let a = self.block(body, cx);
                let b = self.block(orelse, cx);
                return a || b;
            }
            StmtKind::While { test, body } => {
                self.expr(test, cx);
                self.block(body, cx);
            }
            StmtKind::ForIter { var, iter, body, .. } => {
                self.expr(iter, cx);
                self.assign(var, Sym::Con(Ty::Obj), cx);
                self.block(body, cx);
            }
            StmtKind::SetItem { obj, index, value } => {
                self.expr(obj, cx);
                self.expr(index, cx);
                self.expr(value, cx);
            }
            StmtKind::SetAttr { obj, value, .. } => {
                self.expr(obj, cx);
                self.expr(value, cx);
            }
            StmtKind::ForRange { var, start, stop, step, body } => {
                for b in [start.as_ref(), Some(stop), step.as_ref()].into_iter().flatten() {
                    let v = self.expr(b, cx);
                    cx.cs.push(Constraint::IsInt(v));
                }
                cx.solve();
                self.assign(var, Sym::Con(Ty::Int), cx);
                self.block(body, cx);
            }
            StmtKind::Try { body, handlers, orelse, finalbody } => {
                let mut falls = self.block(body, cx) && self.block(orelse, cx);
                for h in handlers {
                    for t in &h.types {
                        if let ExcType::Obj(e) = t {
                            self.expr(e, cx);
                        }
                    }
                    if let Some(n) = &h.name {
                        self.assign(n, Sym::Con(Ty::exc()), cx);
                    }
                    falls |= self.block(&h.body, cx);
                }
                return self.block(finalbody, cx) && falls;
            }
            StmtKind::Raise(e) => {
                if let Some(e) = e {
                    self.expr(e, cx);
                }
                return false;
            }
            StmtKind::AssignGlobal { value, .. } => {
                self.expr(value, cx);
            }
            StmtKind::With { ctx, var, body, .. } => {
                self.expr(ctx, cx);
                if let Some(v) = var {
                    self.assign(v, Sym::Con(Ty::Obj), cx);
                }
                return self.block(body, cx);
            }
            _ => {}
        }
        true
    }

    fn expr(&mut self, e: &Expr, cx: &mut FnCx) -> Sym {
        let s = match &e.kind {
            ExprKind::Int(_) => Sym::Con(Ty::Int),
            ExprKind::Float(_) => Sym::Con(Ty::Float),
            ExprKind::Str(_) => Sym::Con(Ty::Str),
            ExprKind::Bool(_) => Sym::Con(Ty::Bool),
            ExprKind::NoneLit => Sym::Con(Ty::None),
            ExprKind::Local(n) => match cx.env.get(n) {
                Some(&s) => s,
                None => {
                    let v = self.fresh();
                    cx.env.insert(n.clone(), v);
                    v
                }
            },
            ExprKind::BinOp { op, left, right } => {
                let (a, b) = (self.expr(left, cx), self.expr(right, cx));
                let lit = if let ExprKind::Int(v) = right.kind { Some(v) } else { None };
                match (cx.resolve(a), cx.resolve(b)) {
                    (Sym::Con(x), Sym::Con(y)) => ops::binop(*op, x, y, lit).map(Sym::Con).unwrap_or_else(|| self.fresh()),
                    _ => {
                        let r = self.fresh();
                        cx.cs.push(Constraint::Bin(*op, a, b, r));
                        r
                    }
                }
            }
            ExprKind::Unary { op, operand } => {
                let a = self.expr(operand, cx);
                match op {
                    UnaryOp::Not => Sym::Con(Ty::Bool),
                    _ => {
                        let r = self.fresh();
                        let n = match op {
                            UnaryOp::Neg => "Neg",
                            UnaryOp::Pos => "Pos",
                            _ => "Invert",
                        };
                        cx.cs.push(Constraint::Unary(n, a, r));
                        r
                    }
                }
            }
            ExprKind::Compare { first, rest } => {
                let mut l = self.expr(first, cx);
                for (op, c) in rest {
                    let r = self.expr(c, cx);
                    let n = match op {
                        CmpOp::Eq | CmpOp::NotEq | CmpOp::Is | CmpOp::IsNot => None,
                        CmpOp::In | CmpOp::NotIn => Some("Contains"),
                        CmpOp::Lt => Some("Lt"),
                        CmpOp::LtE => Some("Le"),
                        CmpOp::Gt => Some("Gt"),
                        CmpOp::GtE => Some("Ge"),
                    };
                    if let Some(n) = n {
                        if matches!(cx.resolve(l), Sym::Var(_)) || matches!(cx.resolve(r), Sym::Var(_)) {
                            cx.cs.push(Constraint::Cmp(n, l, r));
                        }
                    }
                    l = r;
                }
                Sym::Con(Ty::Bool)
            }
            ExprKind::BoolOp { values, .. } => {
                let syms: Vec<Sym> = values.iter().map(|v| self.expr(v, cx)).collect();
                for s in &syms[1..] {
                    cx.cs.push(Constraint::Same(syms[0], *s));
                }
                syms[0]
            }
            ExprKind::IfExp { test, body, orelse } => {
                self.expr(test, cx);
                let a = self.expr(body, cx);
                let b = self.expr(orelse, cx);
                cx.cs.push(Constraint::Same(a, b));
                a
            }
            ExprKind::ExternCall { ext, func, args } => {
                for a in args {
                    self.expr(a, cx);
                }
                Sym::Con(self.m.externs[*ext].funcs[*func].ret.ty())
            }
            ExprKind::SuperDispatch { default, .. } => self.expr(default, cx),
            ExprKind::Walrus { target, global, value } => {
                let v = self.expr(value, cx);
                if global.is_none() {
                    self.assign(target, v, cx);
                }
                v
            }
            ExprKind::Annotated { ty, value } => {
                self.expr(value, cx);
                Sym::Con(*ty)
            }
            ExprKind::Slot(_) | ExprKind::PyBuiltin(_) => Sym::Con(Ty::Obj),
            ExprKind::Await(v) | ExprKind::Yield(Some(v)) | ExprKind::YieldFrom(v) => {
                self.expr(v, cx);
                Sym::Con(Ty::Obj)
            }
            ExprKind::Yield(None) => Sym::Con(Ty::None),
            ExprKind::Global { .. } => self.fresh(),
            ExprKind::Slice { lo, hi, step } => {
                for p in [lo, hi, step].into_iter().flatten() {
                    self.expr(p, cx);
                }
                Sym::Con(Ty::Obj)
            }
            ExprKind::Set(items) => {
                for a in items {
                    self.expr(a, cx);
                }
                self.fresh()
            }
            ExprKind::Unpack { value, .. } | ExprKind::Starred(value) => {
                self.expr(value, cx);
                self.fresh()
            }
            ExprKind::Comp { .. } | ExprKind::ClassRef(_) | ExprKind::MakeClosure { .. } | ExprKind::EnvDefault(_) | ExprKind::EnvRead { .. } => self.fresh(),
            ExprKind::New { args, kwargs, .. } => {
                for a in args.iter().chain(kwargs.iter().map(|(_, v)| v)) {
                    self.expr(a, cx);
                }
                self.fresh()
            }
            ExprKind::NewExc { args, .. } => {
                for a in args {
                    self.expr(a, cx);
                }
                Sym::Con(Ty::exc())
            }
            ExprKind::GetAttr { obj, .. } => {
                self.expr(obj, cx);
                Sym::Con(Ty::Obj)
            }
            ExprKind::CallObj { func, args, kwargs } => {
                self.expr(func, cx);
                for a in args.iter().chain(kwargs.iter().map(|(_, v)| v)) {
                    self.expr(a, cx);
                }
                Sym::Con(Ty::Obj)
            }
            ExprKind::List(items) | ExprKind::Tuple(items) => {
                for a in items {
                    self.expr(a, cx);
                }
                self.fresh()
            }
            ExprKind::Dict(pairs) => {
                for (k, v) in pairs {
                    self.expr(k, cx);
                    self.expr(v, cx);
                }
                Sym::Con(Ty::Obj)
            }
            ExprKind::Subscript { obj, index } => {
                self.expr(obj, cx);
                self.expr(index, cx);
                Sym::Con(Ty::Obj)
            }
            ExprKind::Call { callee, args, .. } => {
                let syms: Vec<Sym> = args.iter().map(|a| self.expr(a, cx)).collect();
                match callee {
                    Callee::Builtin(b) => match b {
                        Builtin::Print | Builtin::PrintKw => Sym::Con(Ty::None),
                        Builtin::Int | Builtin::Len => Sym::Con(Ty::Int),
                        Builtin::Float => Sym::Con(Ty::Float),
                        Builtin::Str => Sym::Con(Ty::Str),
                        Builtin::Bool => Sym::Con(Ty::Bool),
                        Builtin::Abs => {
                            let r = self.fresh();
                            if let Some(&a) = syms.first() {
                                cx.cs.push(Constraint::Unary("Abs", a, r));
                            }
                            r
                        }
                        Builtin::Min | Builtin::Max => {
                            for s in syms.iter().skip(1) {
                                cx.cs.push(Constraint::Same(syms[0], *s));
                            }
                            syms.first().copied().unwrap_or(Sym::Con(Ty::None))
                        }
                    },
                    Callee::Func(fid) => {
                        if let Some((_, ps, r)) = self.in_progress.last().filter(|(f, ..)| f == fid).cloned() {
                            // Direct recursion: the call uses the function's own signature.
                            for (p, a) in ps.iter().zip(&syms) {
                                cx.bind(*p, *a);
                            }
                            cx.solve();
                            r
                        } else if self.in_progress.iter().any(|(f, ..)| f == fid) {
                            let r = self.fresh();
                            cx.cs.push(Constraint::Rec(self.m.funcs[*fid].name.clone(), syms, r));
                            r
                        } else {
                            let sch = self.scheme(*fid);
                            self.instantiate(&sch, &syms, cx)
                        }
                    }
                }
            }
        };
        cx.solve();
        cx.resolve(s)
    }

    fn instantiate(&mut self, sch: &Scheme, args: &[Sym], cx: &mut FnCx) -> Sym {
        let mut map: HashMap<u32, Sym> = HashMap::new();
        let mut rn = |s: Sym, me: &mut Self| -> Sym {
            match s {
                Sym::Var(v) => *map.entry(v).or_insert_with(|| me.fresh()),
                c => c,
            }
        };
        let params: Vec<Sym> = sch.params.iter().map(|&p| rn(p, self)).collect();
        let ret = rn(sch.ret, self);
        let cs: Vec<Constraint> = sch
            .constraints
            .iter()
            .map(|c| match c {
                Constraint::Bin(o, a, b, r) => Constraint::Bin(*o, rn(*a, self), rn(*b, self), rn(*r, self)),
                Constraint::Unary(n, a, r) => Constraint::Unary(n, rn(*a, self), rn(*r, self)),
                Constraint::Cmp(n, a, b) => Constraint::Cmp(n, rn(*a, self), rn(*b, self)),
                Constraint::Same(a, b) => Constraint::Same(rn(*a, self), rn(*b, self)),
                Constraint::IsInt(a) => Constraint::IsInt(rn(*a, self)),
                Constraint::Rec(n, a, r) => Constraint::Rec(n.clone(), a.iter().map(|x| rn(*x, self)).collect(), rn(*r, self)),
            })
            .collect();
        for (p, a) in params.iter().zip(args) {
            cx.bind(*p, *a);
        }
        cx.cs.extend(cs);
        cx.solve();
        cx.resolve(ret)
    }
}

fn subst_c(cx: &FnCx, c: &Constraint) -> Constraint {
    let r = |s: &Sym| cx.resolve(*s);
    match c {
        Constraint::Bin(o, a, b, x) => Constraint::Bin(*o, r(a), r(b), r(x)),
        Constraint::Unary(n, a, x) => Constraint::Unary(n, r(a), r(x)),
        Constraint::Cmp(n, a, b) => Constraint::Cmp(n, r(a), r(b)),
        Constraint::Same(a, b) => Constraint::Same(r(a), r(b)),
        Constraint::IsInt(a) => Constraint::IsInt(r(a)),
        Constraint::Rec(n, a, x) => Constraint::Rec(n.clone(), a.iter().map(r).collect(), r(x)),
    }
}

/// Render a scheme as `name(A, B) -> R where Add(A, B, R)`.
pub fn render(name: &str, s: &Scheme) -> String {
    let mut names: HashMap<u32, String> = HashMap::new();
    let letters = ["A", "B", "C", "D", "E", "F", "G", "H"];
    let mut pi = 0;
    for p in &s.params {
        if let Sym::Var(v) = p {
            if !names.contains_key(v) {
                names.insert(*v, letters.get(pi).map(|s| s.to_string()).unwrap_or_else(|| format!("T{pi}")));
                pi += 1;
            }
        }
    }
    let mut ri = 0;
    let mut nm = |s: &Sym, names: &mut HashMap<u32, String>| -> String {
        match s {
            Sym::Con(t) => t.py_name().to_string(),
            Sym::Var(v) => names
                .entry(*v)
                .or_insert_with(|| {
                    ri += 1;
                    if ri == 1 { "R".to_string() } else { format!("R{}", ri - 1) }
                })
                .clone(),
        }
    };
    let ps: Vec<String> = s.params.iter().map(|p| nm(p, &mut names)).collect();
    let mut ret = nm(&s.ret, &mut names);
    // Concrete conflicts with the return type read as a union: `-> int | None`.
    for c in &s.constraints {
        if let Constraint::Same(Sym::Con(a), Sym::Con(b)) = c {
            if Sym::Con(*a) == s.ret && !ret.split(" | ").any(|x| x == b.py_name()) {
                ret = format!("{ret} | {}", b.py_name());
            }
        }
    }
    let mut cs = Vec::new();
    for c in &s.constraints {
        if matches!(c, Constraint::Same(Sym::Con(_), Sym::Con(_))) {
            continue;
        }
        cs.push(match c {
            Constraint::Bin(o, a, b, r) => format!("{}({}, {}, {})", o.trait_name(), nm(a, &mut names), nm(b, &mut names), nm(r, &mut names)),
            Constraint::Unary(n, a, r) => format!("{n}({}, {})", nm(a, &mut names), nm(r, &mut names)),
            Constraint::Cmp(n, a, b) => format!("{n}({}, {})", nm(a, &mut names), nm(b, &mut names)),
            Constraint::Same(a, b) => format!("{} = {}", nm(a, &mut names), nm(b, &mut names)),
            Constraint::IsInt(a) => format!("{}: int", nm(a, &mut names)),
            Constraint::Rec(f, a, r) => format!(
                "{f}({}) -> {}",
                a.iter().map(|x| nm(x, &mut names)).collect::<Vec<_>>().join(", "),
                nm(r, &mut names)
            ),
        });
    }
    let mut out = format!("{name}({}) -> {ret}", ps.join(", "));
    if !cs.is_empty() {
        out.push_str(" where ");
        out.push_str(&cs.join(", "));
    }
    out
}
