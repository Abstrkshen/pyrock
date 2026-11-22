//! Definite-assignment analysis. A local read that is not provably preceded
//! by an assignment on every path is "maybe unbound": codegen stores it as
//! `Option<T>` and raises UnboundLocalError/NameError exactly like CPython.

use pyfast_ir::*;
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
        StmtKind::ForIter { var, iter, body } => {
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
        ExprKind::Unary { operand, .. } => expr(operand, cur, out),
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
        ExprKind::GetAttr { obj, .. } | ExprKind::Annotated { value: obj, .. } => expr(obj, cur, out),
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
        _ => {}
    }
}
