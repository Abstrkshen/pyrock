//! The Python-semantic operator table: which concrete operand types an
//! operator accepts and what type it produces. Codegen and inference both
//! consult this table so they can never disagree.

use pyfast_ir::{BinOp, Builtin, CmpOp, Ty, UnaryOp};

fn promote(l: Ty, r: Ty) -> Option<Ty> {
    if !l.is_numeric() || !r.is_numeric() {
        return None;
    }
    Some(if l == Ty::Float || r == Ty::Float { Ty::Float } else { Ty::Int })
}

fn intlike(t: Ty) -> bool {
    matches!(t, Ty::Int | Ty::Bool)
}

/// Result type of `l op r`. `rhs_literal` is the right operand's value when
/// it is an int literal (needed for `int ** int`, whose type is value-dependent).
/// `T | None` used where a `T` is needed: checked at runtime (TypeError on None).
pub fn base(t: Ty) -> Ty {
    t.as_opt().unwrap_or(t)
}

pub fn binop(op: BinOp, l: Ty, r: Ty, rhs_literal: Option<i64>) -> Option<Ty> {
    use BinOp::*;
    let (l, r) = (base(l), base(r));
    if l == Ty::Obj || r == Ty::Obj {
        return Some(Ty::Obj); // CPython operator protocol
    }
    // `int | float` operands: the result's kind is only known at run time.
    if (l.is_num() || r.is_num()) && l.is_number() && r.is_number() {
        return match op {
            Add | Sub | Mul | FloorDiv | Mod | Pow => Some(Ty::num()),
            Div => Some(Ty::Float),
            _ => None,
        };
    }
    match op {
        // No native type supports `@` (TypeError, as in CPython).
        MatMul => None,
        Add => {
            if l == Ty::Str && r == Ty::Str {
                Some(Ty::Str)
            } else {
                promote(l, r)
            }
        }
        Sub | FloorDiv | Mod => promote(l, r),
        Mul => match (l, r) {
            (Ty::Str, t) | (t, Ty::Str) if intlike(t) => Some(Ty::Str),
            _ => promote(l, r),
        },
        Div => promote(l, r).map(|_| Ty::Float),
        Pow => match promote(l, r)? {
            Ty::Float => Some(Ty::Float),
            // int ** int is a float when the exponent is negative. With a
            // literal exponent the type is known; otherwise the int
            // specialization raises at runtime for negative exponents
            // (documented M1 divergence, see docs/subset.md).
            _ => match rhs_literal {
                Some(v) if v < 0 => Some(Ty::Float),
                _ => Some(Ty::Int),
            },
        },
        BitAnd | BitOr | BitXor => {
            if l == Ty::Bool && r == Ty::Bool {
                Some(Ty::Bool)
            } else if intlike(l) && intlike(r) {
                Some(Ty::Int)
            } else {
                None
            }
        }
        LShift | RShift => (intlike(l) && intlike(r)).then_some(Ty::Int),
    }
}

/// Whether a value of type `v` satisfies annotation `ann` (PEP 484 numeric
/// tower: `bool` is an `int`, `int` is acceptable for `float`).
pub fn satisfies(v: Ty, ann: Ty) -> bool {
    v == ann || ann == Ty::Obj || matches!((v, ann), (Ty::Bool, Ty::Int) | (Ty::Bool | Ty::Int, Ty::Float))
}

pub fn unary(op: UnaryOp, t: Ty) -> Option<Ty> {
    let t = if op == UnaryOp::Not { t } else { base(t) };
    match op {
        UnaryOp::Not => Some(Ty::Bool),
        _ if t == Ty::Obj => Some(Ty::Obj),
        UnaryOp::Neg | UnaryOp::Pos => match t {
            Ty::Bool | Ty::Int => Some(Ty::Int),
            Ty::Float => Some(Ty::Float),
            t if t.is_num() => Some(t),
            _ => None,
        },
        UnaryOp::Invert => intlike(t).then_some(Ty::Int),
    }
}

pub fn compare(op: CmpOp, l: Ty, r: Ty) -> bool {
    if l == Ty::Obj || r == Ty::Obj {
        return true;
    }
    if (base(l).is_num() || base(r).is_num()) && base(l).is_number() && base(r).is_number() {
        return !matches!(op, CmpOp::In | CmpOp::NotIn);
    }
    let optional = |t: Ty| t == Ty::None || t.as_opt().is_some();
    if l.as_comp().is_some_and(|c| !matches!(c, pyfast_ir::Comp::Opt(_))) || r.as_comp().is_some_and(|c| !matches!(c, pyfast_ir::Comp::Opt(_))) {
        return match op {
            CmpOp::Eq | CmpOp::NotEq | CmpOp::Is | CmpOp::IsNot => true,
            CmpOp::In | CmpOp::NotIn => crate::native::contains_ok(l, r),
            // Ordering: same-shaped sequences (lists, tuples), set inclusion.
            _ => {
                let seq = |t: Ty| t.list_elem().is_some() || t.tuple_items().is_some() || matches!(t.as_comp(), Some(pyfast_ir::Comp::TupleVar(_)));
                (l == r && (seq(l) || l.set_elem().is_some())) || (seq(l) && seq(r) && crate::native::ty_join(l, r, false).is_some())
            }
        };
    }
    match op {
        CmpOp::Eq | CmpOp::NotEq => true,
        CmpOp::In | CmpOp::NotIn if r.as_comp().is_some() => crate::native::contains_ok(l, r),
        CmpOp::Is | CmpOp::IsNot if optional(l) || optional(r) => true,
        _ if l.as_opt().is_some() || r.as_opt().is_some() => compare(op, base(l), base(r)),
        CmpOp::In | CmpOp::NotIn => l == Ty::Str && r == Ty::Str,
        // `x is None` on a native value has a statically known answer.
        CmpOp::Is | CmpOp::IsNot => l == Ty::None || r == Ty::None,
        _ => (l.is_numeric() && r.is_numeric()) || (l == Ty::Str && r == Ty::Str),
    }
}

/// Result type of a builtin call, or an error message (Python's TypeError text).
pub fn builtin(b: Builtin, args: &[Ty], boxing: bool) -> Result<Ty, String> {
    let lifted: Vec<Ty> = args.iter().map(|t| base(*t)).collect();
    let args: &[Ty] = if matches!(b, Builtin::Print | Builtin::PrintKw | Builtin::Str | Builtin::Bool) { args } else { &lifted };
    let any_obj = args.contains(&Ty::Obj);
    let arity = |n: usize| -> Result<(), String> {
        if args.len() == n {
            Ok(())
        } else {
            Err(format!("{}() takes exactly {n} argument(s) ({} given)", b.name(), args.len()))
        }
    };
    match b {
        Builtin::Print => Ok(Ty::None),
        Builtin::PrintKw => {
            let n = args.len();
            for (i, what) in [(n - 3, "sep"), (n - 2, "end")] {
                if !matches!(args[i], Ty::Str | Ty::None | Ty::Obj) {
                    return Err(format!("{what} must be None or a string, not {}", args[i]));
                }
            }
            Ok(Ty::None)
        }
        Builtin::Abs => {
            arity(1)?;
            if any_obj {
                return Ok(Ty::Obj);
            }
            unary(UnaryOp::Neg, args[0]).ok_or_else(|| format!("bad operand type for abs(): '{}'", args[0]))
        }
        Builtin::Int | Builtin::Float => {
            if args.is_empty() {
                return Ok(if b == Builtin::Int { Ty::Int } else { Ty::Float });
            }
            arity(1)?;
            match args[0] {
                Ty::None => Err(format!("{}() argument must be a string or a real number, not 'NoneType'", b.name())),
                _ => Ok(if b == Builtin::Int { Ty::Int } else { Ty::Float }),
            }
        }
        Builtin::Str | Builtin::Bool => {
            if args.len() > 1 {
                return Err(format!("{}() takes at most 1 argument", b.name()));
            }
            Ok(if b == Builtin::Str { Ty::Str } else { Ty::Bool })
        }
        Builtin::Len => {
            arity(1)?;
            let sized = args[0].as_comp().is_some_and(|c| matches!(c, pyfast_ir::Comp::List(_) | pyfast_ir::Comp::Dict(..) | pyfast_ir::Comp::Set(_) | pyfast_ir::Comp::Tuple(_) | pyfast_ir::Comp::TupleVar(_) | pyfast_ir::Comp::View(..) | pyfast_ir::Comp::Record(_)));
            if args[0] == Ty::Str || args[0] == Ty::Obj || sized {
                Ok(Ty::Int)
            } else {
                Err(format!("object of type '{}' has no len()", args[0]))
            }
        }
        Builtin::Min | Builtin::Max => {
            if args.len() == 1 && !any_obj {
                if let Some(e) = crate::native::iter_elem(args[0]) {
                    if e.is_never() {
                        return Err(format!("{}() iterable argument is empty", b.name()));
                    }
                    return Ok(e);
                }
            }
            if args.len() < 2 && (any_obj || boxing) {
                return Ok(Ty::Obj); // min(iterable): CPython builtin
            }
            if args.len() < 2 {
                return Err(format!("{}() with fewer than 2 positional arguments needs an iterable (not supported yet)", b.name()));
            }
            if args.iter().all(|&t| t == args[0]) && (args[0].is_numeric() || args[0] == Ty::Str) {
                Ok(args[0])
            } else if args.iter().all(|t| matches!(t, Ty::Int | Ty::Float) || t.is_num()) {
                Ok(Ty::num()) // max(3, 2.5) is the int 3, max(2, 2.5) the float 2.5
            } else if any_obj || boxing {
                Ok(Ty::Obj) // CPython's own min/max over boxed values
            } else {
                Err(format!(
                    "{}() over mixed types ({}) returns a value-dependent type; unions are milestone M4",
                    b.name(),
                    args.iter().map(|t| t.py_name()).collect::<Vec<_>>().join(", ")
                ))
            }
        }
    }
}
