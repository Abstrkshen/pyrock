//! Walking the HIR: every statement and expression of a function body.

use crate::hir::*;

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

/// The expressions a statement evaluates directly (not those of nested blocks).
pub fn stmt_exprs(s: &Stmt) -> Vec<&Expr> {
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

pub fn walk_expr(e: &Expr, f: &mut impl FnMut(&Expr), skip_callee: bool) {
    f(e);
    match &e.kind {
        ExprKind::BinOp { left, right, .. } => {
            walk_expr(left, f, skip_callee);
            walk_expr(right, f, skip_callee);
        }
        ExprKind::Unary { operand, .. } | ExprKind::Await(operand) | ExprKind::Yield(Some(operand)) | ExprKind::YieldFrom(operand) => walk_expr(operand, f, skip_callee),
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
        ExprKind::GetAttr { obj, .. } | ExprKind::Annotated { value: obj, .. } | ExprKind::Walrus { value: obj, .. } => walk_expr(obj, f, skip_callee),
        ExprKind::SuperDispatch { alts, default, .. } => {
            alts.iter().for_each(|a| walk_expr(&a.1, f, skip_callee));
            walk_expr(default, f, skip_callee)
        }
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
