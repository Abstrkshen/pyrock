//! Definite-assignment analysis. A local read that is not provably preceded
//! by an assignment on every path is "maybe unbound": codegen stores it as
//! `Option<T>` and raises UnboundLocalError/NameError exactly like CPython.

use pyrock_ir::*;
use std::collections::HashSet;

type Set = HashSet<String>;

pub fn maybe_unbound(f: &Function) -> HashSet<String> {
    let mut out = HashSet::new();
    let init: Set = f.params.iter().cloned().collect();
    block(&f.body, init, &mut out);
    out
}

/// Returns the definitely-assigned set after the block, or `None` if the
/// block never falls through.
fn block(stmts: &[Stmt], mut cur: Set, out: &mut HashSet<String>) -> Option<Set> {
    for s in stmts {
        cur = stmt(s, cur, out)?;
    }
    Some(cur)
}

fn stmt(s: &Stmt, mut cur: Set, out: &mut HashSet<String>) -> Option<Set> {
    match &s.kind {
        StmtKind::Assign { target, value } => {
            expr(value, &cur, out);
            cur.insert(target.clone());
        }
        StmtKind::AugAssign { target, value, .. } => {
            if !cur.contains(target) {
                out.insert(target.clone());
            }
            expr(value, &cur, out);
            cur.insert(target.clone());
        }
        StmtKind::Expr(e) => expr(e, &cur, out),
        StmtKind::Return(v) => {
            if let Some(e) = v {
                expr(e, &cur, out);
            }
            return None;
        }
        StmtKind::Break | StmtKind::Continue => return None,
        StmtKind::Pass | StmtKind::Import(_) => {}
        StmtKind::ForIter { var, iter, body, .. } => {
            expr(iter, &cur, out);
            let mut inner = cur.clone();
            inner.insert(var.clone());
            block(body, inner, out);
        }
        StmtKind::SetItem { obj, index, value } => {
            expr(obj, &cur, out);
            expr(index, &cur, out);
            expr(value, &cur, out);
        }
        StmtKind::SetAttr { obj, value, .. } => {
            expr(obj, &cur, out);
            expr(value, &cur, out);
        }
        // A constant test runs one branch (synthetic `if True:` blocks hold unpacking).
        StmtKind::If { test, body, .. } if matches!(test.kind, ExprKind::Bool(true)) => return block(body, cur, out),
        StmtKind::If { test, orelse, .. } if matches!(test.kind, ExprKind::Bool(false)) => return block(orelse, cur, out),
        StmtKind::If { test, body, orelse } => {
            expr(test, &cur, out);
            let a = block(body, cur.clone(), out);
            let b = block(orelse, cur, out);
            return match (a, b) {
                (Some(a), Some(b)) => Some(a.intersection(&b).cloned().collect()),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
        }
        StmtKind::While { test, body } => {
            expr(test, &cur, out);
            block(body, cur.clone(), out);
        }
        StmtKind::Raise(e) => {
            if let Some(e) = e {
                expr(e, &cur, out);
            }
            return None;
        }
        StmtKind::AssignGlobal { value, .. } => expr(value, &cur, out),
        StmtKind::Del(t) => {
            expr(t, &cur, out);
            // `del x` unbinds it: later reads may see it unbound.
            if let ExprKind::Local(n) = &t.kind {
                cur.remove(n);
            }
        }
        StmtKind::With { ctx, var, body, .. } => {
            expr(ctx, &cur, out);
            let mut inner = cur.clone();
            if let Some(v) = var {
                inner.insert(v.clone());
            }
            // The body may be cut short by an exception that __exit__ suppresses.
            block(body, inner, out);
        }
        StmtKind::Try { body, handlers, orelse, finalbody } => {
            // Any statement of the body may raise: handlers start from the
            // state before the `try`.
            let mut outs = Vec::new();
            if let Some(b) = block(body, cur.clone(), out) {
                outs.extend(block(orelse, b, out));
            }
            for h in handlers {
                for t in &h.types {
                    if let ExcType::Obj(e) = t {
                        expr(e, &cur, out);
                    }
                }
                let mut inner = cur.clone();
                if let Some(n) = &h.name {
                    inner.insert(n.clone());
                }
                outs.extend(block(&h.body, inner, out));
            }
            // `finally` also runs on the exceptional path: assume only `cur`.
            block(finalbody, cur.clone(), out)?;
            let mut it = outs.into_iter();
            let first = it.next()?;
            let merged = it.fold(first, |a, b| a.intersection(&b).cloned().collect());
            let fin = block(finalbody, merged, out)?;
            return Some(fin);
        }
        StmtKind::ForRange { var, start, stop, step, body } => {
            for b in [start.as_ref(), Some(stop), step.as_ref()].into_iter().flatten() {
                expr(b, &cur, out);
            }
            let mut inner = cur.clone();
            inner.insert(var.clone());
            block(body, inner, out);
        }
    }
    Some(cur)
}

fn expr(e: &Expr, cur: &Set, out: &mut HashSet<String>) {
    match &e.kind {
        ExprKind::Local(n) => {
            if !cur.contains(n) {
                out.insert(n.clone());
            }
        }
        ExprKind::BinOp { left, right, .. } => {
            expr(left, cur, out);
            expr(right, cur, out);
        }
        ExprKind::Unary { operand, .. } | ExprKind::Await(operand) | ExprKind::Yield(Some(operand)) | ExprKind::YieldFrom(operand) => expr(operand, cur, out),
        ExprKind::Compare { first, rest } => {
            expr(first, cur, out);
            for (_, c) in rest {
                expr(c, cur, out);
            }
        }
        ExprKind::BoolOp { values, .. } => values.iter().for_each(|v| expr(v, cur, out)),
        ExprKind::IfExp { test, body, orelse } => {
            expr(test, cur, out);
            expr(body, cur, out);
            expr(orelse, cur, out);
        }
        ExprKind::Call { args, .. } | ExprKind::ExternCall { args, .. } | ExprKind::List(args) | ExprKind::Tuple(args) => {
            args.iter().for_each(|a| expr(a, cur, out))
        }
        ExprKind::GetAttr { obj, .. } | ExprKind::Annotated { value: obj, .. } | ExprKind::Walrus { value: obj, .. } => expr(obj, cur, out),
        ExprKind::CallObj { func, args, kwargs } => {
            expr(func, cur, out);
            args.iter().for_each(|a| expr(a, cur, out));
            kwargs.iter().for_each(|(_, a)| expr(a, cur, out));
        }
        ExprKind::Dict(pairs) => pairs.iter().for_each(|(k, v)| {
            expr(k, cur, out);
            expr(v, cur, out);
        }),
        ExprKind::Subscript { obj, index } => {
            expr(obj, cur, out);
            expr(index, cur, out);
        }
        ExprKind::NewExc { args, .. } | ExprKind::Set(args) => args.iter().for_each(|a| expr(a, cur, out)),
        ExprKind::Slice { lo, hi, step } => [lo, hi, step].into_iter().flatten().for_each(|a| expr(a, cur, out)),
        ExprKind::Unpack { value, .. } | ExprKind::Starred(value) => expr(value, cur, out),
        ExprKind::New { args, kwargs, .. } => args.iter().chain(kwargs.iter().map(|(_, v)| v)).for_each(|a| expr(a, cur, out)),
        ExprKind::MakeClosure { defaults, .. } => defaults.iter().for_each(|a| expr(a, cur, out)),
        ExprKind::EnvRead { value, .. } => expr(value, cur, out),
        ExprKind::Comp { elt, val, gens, .. } => {
            let mut inner = cur.clone();
            for g in gens {
                expr(&g.iter, &inner, out);
                inner.insert(g.var.clone());
                if let Some(after) = block(&g.unpack, inner.clone(), out) {
                    inner = after;
                }
                for c in &g.ifs {
                    expr(c, &inner, out);
                }
            }
            expr(elt, &inner, out);
            if let Some(v) = val {
                expr(v, &inner, out);
            }
        }
        _ => {}
    }
}
