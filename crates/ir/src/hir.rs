//! Structured, scope-resolved HIR. Python evaluation order is preserved:
//! nothing here reorders sub-expressions.

use crate::{ClassId, Span, Ty};

pub type FuncId = usize;

#[derive(Clone, Debug)]
pub struct Module {
    pub file: String,
    /// All functions of all natively compiled Python units; `funcs[main]`
    /// is the synthetic module-body function of the entry file.
    pub funcs: Vec<Function>,
    pub main: FuncId,
    /// Natively compiled Python source files (the entry file is unit 0).
    pub units: Vec<Unit>,
    /// Rust and C modules linked into the binary.
    pub externs: Vec<ExternModule>,
    /// Names bound by CPython imports, filled at their import statement.
    pub slots: Vec<PySlot>,
    /// Directory of the entry file (prepended to the embedded `sys.path`).
    pub root_dir: String,
    /// How each import was resolved, for the explain report.
    pub imports: Vec<ImportRecord>,
    /// Module-level variables stored in global cells: (unit, name).
    pub globals: Vec<(usize, String)>,
    /// Natively compiled classes.
    pub classes: Vec<ClassDef>,
}

impl Module {
    pub fn files(&self) -> Vec<String> {
        self.units.iter().map(|u| u.path.clone()).collect()
    }

    pub fn uses_python(&self) -> bool {
        !self.slots.is_empty()
    }
}

/// A Python source file compiled natively.
#[derive(Clone, Debug)]
pub struct Unit {
    pub name: String,
    pub path: String,
    /// The unit's module-body function (runs once, at its first import).
    pub init: FuncId,
}

/// How an `import` was resolved.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ImportRecord {
    pub name: String,
    pub kind: &'static str,
    pub path: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PySlot {
    /// The Python name the import binds (`json`, `np`, `sqrt`).
    pub binding: String,
    pub kind: SlotKind,
}

#[derive(Clone, Debug)]
pub enum SlotKind {
    /// `import a.b` (binds the top package `a`) or `import a.b as x` (binds `a.b`).
    Import { module: String, bind_top: bool },
    /// `from module import attr`
    From { module: String, attr: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Rust,
    C,
}

#[derive(Clone, Debug)]
pub struct ExternModule {
    pub name: String,
    pub lang: Lang,
    /// Absolute path of the `.rs` / `.c` source.
    pub path: String,
    /// Absolute path of the C header the signatures came from, if any.
    pub header: Option<String>,
    pub funcs: Vec<ExternFn>,
    /// Functions present in the source whose signatures cannot cross the boundary.
    pub skipped: Vec<(String, String)>,
    /// `pub struct`s (Rust) / `typedef struct` (C) usable from Python.
    pub structs: Vec<ExternStruct>,
    /// Build settings from `pyfast:` comments: Cargo dependencies (`name = "1"`)
    /// for Rust, libraries / pkg-config packages for C.
    pub deps: Vec<String>,
    pub link_libs: Vec<String>,
    pub pkg_config: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ExternStruct {
    pub name: String,
    /// `pub` fields (C: all fields).
    pub fields: Vec<(String, ExtTy)>,
    pub debug: bool,
    pub clone: bool,
}

#[derive(Clone, Debug)]
pub struct ExternFn {
    /// Free functions: `name`; methods and associated functions: `Struct::name`.
    pub name: String,
    pub params: Vec<ExtTy>,
    pub ret: ExtTy,
    /// Rust `Result<T, E>` return: errors raise `RuntimeError(str(e))`.
    pub fallible: bool,
    /// The struct an `impl` item belongs to, and how it takes `self` (None:
    /// an associated function such as `new`).
    pub owner: Option<usize>,
    pub recv: Option<Pass>,
}

impl ExternFn {
    /// The name without the `Struct::` prefix.
    pub fn short(&self) -> &str {
        self.name.rsplit("::").next().unwrap_or(&self.name)
    }
}

/// How a Rust struct value crosses: by value (cloned), `&S`, or `&mut S`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pass {
    Value,
    Ref,
    Mut,
}

/// A native type at the Rust/C boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtTy {
    /// Any fixed-width integer (`i32`, `u64`, `c_int`, `usize`, ...), by Rust name.
    Int(String),
    F64,
    F32,
    Bool,
    /// Rust `&str`.
    StrRef,
    /// Rust `String`.
    OwnedString,
    /// C `const char *` (NUL-terminated UTF-8).
    CStr,
    Unit,
    /// `Vec<T>` (copied from/to a list).
    Vec(Box<ExtTy>),
    /// `&[T]` (a list or tuple copied into a temporary vector).
    Slice(Box<ExtTy>),
    /// `&mut [T]` / `&mut Vec<T>` (copied in, written back into the same list).
    SliceMut(Box<ExtTy>, bool),
    /// `Option<T>` (`T | None`).
    Opt(Box<ExtTy>),
    /// `(A, B, ...)`.
    Tuple(Vec<ExtTy>),
    /// `HashMap<K, V>` / `BTreeMap<K, V>` (copied from/to a dict); the flag is BTreeMap.
    Map(Box<ExtTy>, Box<ExtTy>, bool),
    /// A struct of an extern module (`ext` index, struct index, Python name).
    Struct { ext: usize, idx: usize, name: String, pass: Pass },
    /// `impl Fn(A..) -> R` / `&dyn Fn(A..) -> R` (a function value).
    Callback(Vec<ExtTy>, Box<ExtTy>, bool),
    /// C `T *data` followed by a length parameter (`size_t n`): one list
    /// argument. Fields: element, mutable (non-const), Rust type of the length.
    CArray(Box<ExtTy>, bool, String),
    /// `Vec<u8>` / `&[u8]` (bytes or a list of ints).
    Bytes(Pass),
    /// C function pointer `R (*f)(A, ...)` (a native function value, called
    /// through a generated trampoline).
    CFn(Vec<ExtTy>, Box<ExtTy>),
}

impl ExtTy {
    pub fn ty(&self) -> Ty {
        match self {
            ExtTy::Int(_) => Ty::Int,
            ExtTy::F64 | ExtTy::F32 => Ty::Float,
            ExtTy::Bool => Ty::Bool,
            ExtTy::StrRef | ExtTy::OwnedString | ExtTy::CStr => Ty::Str,
            ExtTy::Unit => Ty::None,
            ExtTy::Vec(e) | ExtTy::Slice(e) | ExtTy::SliceMut(e, _) | ExtTy::CArray(e, _, _) => Ty::list(e.ty()),
            ExtTy::Opt(e) => Ty::opt(e.ty()),
            ExtTy::Tuple(es) => Ty::tuple(es.iter().map(|e| e.ty()).collect()),
            ExtTy::Map(k, v, _) => Ty::dict(k.ty(), v.ty()),
            ExtTy::Struct { ext, idx, name, .. } => Ty::comp(crate::Comp::ExtStruct(*ext as u32, *idx as u32, Box::leak(name.clone().into_boxed_str()))),
            ExtTy::Callback(..) | ExtTy::CFn(..) => Ty::Obj,
            ExtTy::Bytes(Pass::Value) => Ty::Obj,
            ExtTy::Bytes(_) => Ty::list(Ty::Int),
        }
    }

    pub fn rust(&self) -> String {
        match self {
            ExtTy::Int(t) => t.clone(),
            ExtTy::F64 => "f64".into(),
            ExtTy::F32 => "f32".into(),
            ExtTy::Bool => "bool".into(),
            ExtTy::StrRef => "&str".into(),
            ExtTy::OwnedString => "String".into(),
            ExtTy::CStr => "*const std::ffi::c_char".into(),
            ExtTy::Unit => "()".into(),
            ExtTy::Vec(e) => format!("Vec<{}>", e.rust()),
            ExtTy::Slice(e) => format!("&[{}]", e.rust()),
            ExtTy::SliceMut(e, true) => format!("&mut Vec<{}>", e.rust()),
            ExtTy::SliceMut(e, false) => format!("&mut [{}]", e.rust()),
            ExtTy::Opt(e) => format!("Option<{}>", e.rust()),
            ExtTy::Tuple(es) => format!("({},)", es.iter().map(|e| e.rust()).collect::<Vec<_>>().join(", ")),
            ExtTy::Map(k, v, false) => format!("std::collections::HashMap<{}, {}>", k.rust(), v.rust()),
            ExtTy::Map(k, v, true) => format!("std::collections::BTreeMap<{}, {}>", k.rust(), v.rust()),
            ExtTy::Struct { name, .. } => name.clone(),
            ExtTy::Callback(..) => "_".into(),
            ExtTy::CFn(ps, r) => format!("extern \"C\" fn({}){}", ps.iter().map(|p| p.rust()).collect::<Vec<_>>().join(", "), if **r == ExtTy::Unit { String::new() } else { format!(" -> {}", r.rust()) }),
            ExtTy::CArray(e, true, _) => format!("*mut {}", e.rust()),
            ExtTy::CArray(e, false, _) => format!("*const {}", e.rust()),
            ExtTy::Bytes(Pass::Value) => "Vec<u8>".into(),
            ExtTy::Bytes(_) => "&[u8]".into(),
        }
    }

    /// Whether a value of type `t` can be passed for this parameter.
    pub fn accepts(&self, t: Ty) -> bool {
        let t = t.as_opt().filter(|_| !matches!(self, ExtTy::Opt(_))).unwrap_or(t);
        if t == Ty::Obj {
            // Converted at runtime (TypeError on mismatch); handles and callbacks are native only.
            return !matches!(self, ExtTy::Struct { .. } | ExtTy::Callback(..) | ExtTy::CFn(..));
        }
        let elem_ok = |e: &ExtTy, t: Ty| crate::Ty::iter_elem(t).is_some_and(|x| x.is_never() || e.accepts(x));
        match self {
            ExtTy::Int(_) => matches!(t, Ty::Int | Ty::Bool),
            ExtTy::F64 | ExtTy::F32 => t.is_numeric(),
            ExtTy::Bool => t == Ty::Bool,
            ExtTy::StrRef | ExtTy::OwnedString | ExtTy::CStr => t == Ty::Str,
            ExtTy::Unit => false,
            ExtTy::Vec(e) | ExtTy::Slice(e) | ExtTy::CArray(e, false, _) => elem_ok(e, t),
            // Written back: the list's element type must be exactly the Rust one.
            ExtTy::SliceMut(e, _) | ExtTy::CArray(e, true, _) => t.list_elem().is_some_and(|x| x == e.ty() || x.is_never()),
            ExtTy::Opt(e) => t == Ty::None || e.accepts(t.as_opt().unwrap_or(t)),
            ExtTy::Tuple(es) => t.tuple_items().is_some_and(|ts| ts.len() == es.len() && es.iter().zip(&ts).all(|(e, x)| e.accepts(*x))),
            ExtTy::Map(k, v, _) => t.dict_kv().is_some_and(|(a, b)| (a.is_never() || k.accepts(a)) && (b.is_never() || v.accepts(b))),
            ExtTy::Struct { .. } => t == self.ty(),
            ExtTy::Callback(..) => t.as_closure().is_some() || t.union_members().is_some_and(|ms| ms.iter().all(|m| m.as_closure().is_some())),
            ExtTy::Bytes(_) => t.list_elem().is_some_and(|x| matches!(x, Ty::Int | Ty::Bool) || x.is_never()),
            ExtTy::CFn(..) => t.as_closure().is_some(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Function {
    pub id: FuncId,
    pub name: String,
    /// Positional parameters, then keyword-only ones.
    pub params: Vec<String>,
    /// Every local name (params first), in first-binding order.
    pub locals: Vec<String>,
    pub body: Vec<Stmt>,
    pub span: Span,
    pub is_module: bool,
    /// Index into `Module::units`.
    pub unit: usize,
    /// Parameter annotations that name a type pyfast understands.
    pub param_ann: Vec<Option<Ty>>,
    /// Return annotation, if it names a type pyfast understands.
    pub ret_ann: Option<Ty>,
    /// The class this function is a method of.
    pub class: Option<ClassId>,
    /// The enclosing function of a nested function or lambda.
    pub parent: Option<FuncId>,
    /// Variables of enclosing functions this function uses (closure cells,
    /// passed in its environment, in this order).
    pub captures: Vec<String>,
    /// Own locals that nested functions capture (stored in shared cells).
    pub cells: Vec<String>,
    /// How call arguments bind to the parameters.
    pub sig: CallSig,
    /// `async def`: a call makes a coroutine (`Coro<T>`) running the body.
    pub is_async: bool,
}

impl Function {
    /// Rust-safe base name (`helpers.area` -> `helpers_area`). Nested
    /// functions and lambdas get their id appended (names can repeat).
    pub fn rust_base(&self) -> String {
        let base: String = self.name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
        if self.parent.is_some() || self.name.contains('<') {
            format!("{}_f{}", base.trim_matches('_'), self.id)
        } else {
            base
        }
    }
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Assign { target: String, value: Expr },
    AugAssign { target: String, op: BinOp, value: Expr },
    Expr(Expr),
    Return(Option<Expr>),
    If { test: Expr, body: Vec<Stmt>, orelse: Vec<Stmt> },
    While { test: Expr, body: Vec<Stmt> },
    /// `for var in range(start, stop, step)`; bounds are evaluated once, in order.
    ForRange { var: String, start: Option<Expr>, stop: Expr, step: Option<Expr>, body: Vec<Stmt> },
    /// `for var in <python iterable>`: iteration through the CPython bridge.
    /// `for var in iter` (`async for` when `is_async`: `__aiter__`/`__anext__`).
    ForIter { var: String, iter: Expr, body: Vec<Stmt>, is_async: bool },
    /// `obj[index] = value`
    SetItem { obj: Expr, index: Expr, value: Expr },
    /// `obj.name = value`
    SetAttr { obj: Expr, name: String, value: Expr },
    /// Execute an import at its original position (preserves import order).
    Import(ImportAction),
    /// `try` / `except` / `else` / `finally`.
    Try { body: Vec<Stmt>, handlers: Vec<Handler>, orelse: Vec<Stmt>, finalbody: Vec<Stmt> },
    /// `raise`, `raise e`, `raise e from cause` (the cause only affects the traceback body).
    Raise(Option<Expr>),
    /// `with a as x, b as y: body` (one item per statement after lowering).
    /// `with` (`async with` when `is_async`: `__aenter__`/`__aexit__`).
    With { ctx: Expr, var: Option<String>, body: Vec<Stmt>, is_async: bool },
    /// `del obj[index]` / `del obj.attr` (one target per statement).
    Del(Expr),
    /// Assignment to a module-level variable kept in a global cell
    /// (module body, `global x` in a function, or `mod.x = v`).
    AssignGlobal { unit: usize, name: String, value: Expr },
    Break,
    Continue,
    Pass,
}

/// One `except` clause.
#[derive(Clone, Debug)]
pub struct Handler {
    /// Exception types matched; empty for a bare `except:`.
    pub types: Vec<ExcType>,
    pub name: Option<String>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompKind {
    List,
    Set,
    Dict,
    Gen,
}

/// One `for var in iter if cond` clause. `var` is a synthetic local; `unpack`
/// assigns the clause's target names from it.
#[derive(Clone, Debug)]
pub struct CompFor {
    pub var: String,
    pub iter: Expr,
    pub unpack: Vec<Stmt>,
    pub ifs: Vec<Expr>,
}

#[derive(Clone, Debug)]
pub enum ExcType {
    /// A builtin exception class by name (`ValueError`, `OSError`, ...).
    Builtin(String),
    /// Any other expression: a Python exception class, matched with `isinstance`.
    Obj(Expr),
    /// A natively compiled exception class (`class MyError(ValueError)`).
    Native(ClassId),
}

#[derive(Clone, Debug)]
pub enum ImportAction {
    /// Run CPython's import machinery and fill a slot.
    Py(usize),
    /// Run a natively compiled unit's module body (once).
    Unit(FuncId),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    NoneLit,
    Local(String),
    /// Read of a module-level variable kept in a global cell (NameError while unbound).
    Global { unit: usize, name: String },
    /// `args` are in parameter order (keyword arguments and defaults already
    /// placed). `order`, when not empty, lists argument indices in Python's
    /// evaluation (source) order; defaults are not listed (they are literals).
    Call { callee: Callee, args: Vec<Expr>, order: Vec<usize> },
    BinOp { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    Unary { op: UnaryOp, operand: Box<Expr> },
    /// `await e`: runs a native coroutine inline, or suspends on a Python awaitable.
    Await(Box<Expr>),
    /// Chained comparison `a < b <= c`; each middle operand is evaluated once.
    Compare { first: Box<Expr>, rest: Vec<(CmpOp, Expr)> },
    BoolOp { op: BoolOpKind, values: Vec<Expr> },
    IfExp { test: Box<Expr>, body: Box<Expr>, orelse: Box<Expr> },
    /// A value flowing into an annotated slot (`x: int = v`, an annotated
    /// parameter or return). A Python object is unboxed to `ty` with a
    /// checked conversion; a native value keeps its own (runtime) type.
    Annotated { ty: Ty, value: Box<Expr> },
    /// Call into a linked Rust or C module.
    ExternCall { ext: usize, func: usize, args: Vec<Expr> },
    /// --- CPython bridge (all produce `object`) ---
    /// The value bound by a CPython import.
    Slot(usize),
    /// A Python builtin that has no native implementation (`sorted`, `list`, ...).
    PyBuiltin(String),
    GetAttr { obj: Box<Expr>, name: String },
    CallObj { func: Box<Expr>, args: Vec<Expr>, kwargs: Vec<(String, Expr)> },
    List(Vec<Expr>),
    Tuple(Vec<Expr>),
    Dict(Vec<(Expr, Expr)>),
    Subscript { obj: Box<Expr>, index: Box<Expr> },
    /// `a:b:c` inside a subscript.
    Slice { lo: Option<Box<Expr>>, hi: Option<Box<Expr>>, step: Option<Box<Expr>> },
    /// `{a, b}`
    Set(Vec<Expr>),
    /// `*xs` in a call of a Python object or `print`; in a `CallObj`'s
    /// kwargs, the name `**` marks a `**mapping` argument.
    Starred(Box<Expr>),
    /// The value of an unpacking assignment (`a, *b, c = value`): a fixed
    /// tuple of `n` items whose `star` item (if any) is a list. Checks the
    /// length like CPython (ValueError).
    Unpack { value: Box<Expr>, n: usize, star: Option<usize> },
    /// List/set/dict comprehension or generator expression (evaluated eagerly).
    Comp { kind: CompKind, elt: Box<Expr>, val: Option<Box<Expr>>, gens: Vec<CompFor> },
    /// `Point(...)`: a new instance of a natively compiled class (arguments
    /// bind to `__init__`).
    New { class: ClassId, args: Vec<Expr>, kwargs: Vec<(String, Expr)> },
    /// A natively compiled class used as a value (`isinstance(x, Point)`, `cls`).
    ClassRef(ClassId),
    /// A function value: a nested function or lambda with its captured cells,
    /// or a top-level function used as a value (no captures).
    /// `defaults` are the values of default arguments evaluated at creation
    /// (`lambda x, i=i: ...`); they occupy the environment slots after `captures`.
    MakeClosure { func: FuncId, captures: Vec<String>, defaults: Vec<Expr> },
    /// In a signature: the default stored in environment slot `n` of the
    /// called function value (replaced by `EnvRead` when a call is bound).
    EnvDefault(usize),
    /// Environment slot `index` of function value `value`.
    EnvRead { value: Box<Expr>, index: usize },
    /// `ValueError(...)`: a builtin exception instance (native `rt::Exc`).
    NewExc { kind: String, args: Vec<Expr> },
}

impl ExprKind {
    /// Whether evaluating this node crosses into CPython by itself.
    pub fn is_bridge(&self) -> bool {
        matches!(
            self,
            ExprKind::Slot(_)
                | ExprKind::PyBuiltin(_)
                | ExprKind::GetAttr { .. }
                | ExprKind::CallObj { .. }
                | ExprKind::List(_)
                | ExprKind::Tuple(_)
                | ExprKind::Dict(_)
                | ExprKind::Subscript { .. }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Callee {
    Func(FuncId),
    Builtin(Builtin),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    Print,
    Abs,
    Int,
    Float,
    Str,
    Bool,
    Len,
    Min,
    Max,
    /// `print(*items, sep=, end=, flush=)`: the last three arguments are sep, end, flush.
    PrintKw,
}

impl Builtin {
    pub fn from_name(n: &str) -> Option<Builtin> {
        Some(match n {
            "print" => Builtin::Print,
            "abs" => Builtin::Abs,
            "int" => Builtin::Int,
            "float" => Builtin::Float,
            "str" => Builtin::Str,
            "bool" => Builtin::Bool,
            "len" => Builtin::Len,
            "min" => Builtin::Min,
            "max" => Builtin::Max,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Builtin::Print | Builtin::PrintKw => "print",
            Builtin::Abs => "abs",
            Builtin::Int => "int",
            Builtin::Float => "float",
            Builtin::Str => "str",
            Builtin::Bool => "bool",
            Builtin::Len => "len",
            Builtin::Min => "min",
            Builtin::Max => "max",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    LShift,
    RShift,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::FloorDiv => "//",
            BinOp::Mod => "%",
            BinOp::Pow => "**",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::LShift => "<<",
            BinOp::RShift => ">>",
        }
    }

    /// Constraint name used in inferred generic signatures (`Add(A, B, R)`).
    pub fn trait_name(self) -> &'static str {
        match self {
            BinOp::Add => "Add",
            BinOp::Sub => "Sub",
            BinOp::Mul => "Mul",
            BinOp::Div => "TrueDiv",
            BinOp::FloorDiv => "FloorDiv",
            BinOp::Mod => "Mod",
            BinOp::Pow => "Pow",
            BinOp::BitAnd => "BitAnd",
            BinOp::BitOr => "BitOr",
            BinOp::BitXor => "BitXor",
            BinOp::LShift => "LShift",
            BinOp::RShift => "RShift",
        }
    }

    /// Runtime helper name in `pyfast_runtime::ops`.
    pub fn rt_fn(self) -> &'static str {
        match self {
            BinOp::Add => "add",
            BinOp::Sub => "sub",
            BinOp::Mul => "mul",
            BinOp::Div => "truediv",
            BinOp::FloorDiv => "floordiv",
            BinOp::Mod => "modulo",
            BinOp::Pow => "pow",
            BinOp::BitAnd => "bitand",
            BinOp::BitOr => "bitor",
            BinOp::BitXor => "bitxor",
            BinOp::LShift => "lshift",
            BinOp::RShift => "rshift",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Pos,
    Not,
    Invert,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    LtE,
    Gt,
    GtE,
    In,
    NotIn,
    Is,
    IsNot,
}

impl CmpOp {
    pub fn rt_fn(self) -> &'static str {
        match self {
            CmpOp::In => "contains",
            CmpOp::NotIn => "not_contains",
            CmpOp::Is => "is",
            CmpOp::IsNot => "is_not",
            CmpOp::Eq => "eq",
            CmpOp::NotEq => "ne",
            CmpOp::Lt => "lt",
            CmpOp::LtE => "le",
            CmpOp::Gt => "gt",
            CmpOp::GtE => "ge",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOpKind {
    And,
    Or,
}

/// Builtin exception classes (CPython 3.12+), including the `IOError` /
/// `EnvironmentError` aliases. Kept in sync with `pyfast_runtime::exc::HIERARCHY`.
pub const BUILTIN_EXCEPTIONS: &[&str] = &[
    "BaseException", "Exception", "BaseExceptionGroup", "GeneratorExit", "KeyboardInterrupt", "SystemExit",
    "ArithmeticError", "FloatingPointError", "OverflowError", "ZeroDivisionError", "AssertionError", "AttributeError",
    "BufferError", "EOFError", "ExceptionGroup", "ImportError", "ModuleNotFoundError", "LookupError", "IndexError",
    "KeyError", "MemoryError", "NameError", "UnboundLocalError", "OSError", "IOError", "EnvironmentError",
    "BlockingIOError", "ChildProcessError", "ConnectionError", "BrokenPipeError", "ConnectionAbortedError",
    "ConnectionRefusedError", "ConnectionResetError", "FileExistsError", "FileNotFoundError", "InterruptedError",
    "IsADirectoryError", "NotADirectoryError", "PermissionError", "ProcessLookupError", "TimeoutError",
    "ReferenceError", "RuntimeError", "NotImplementedError", "PythonFinalizationError", "RecursionError",
    "StopAsyncIteration", "StopIteration", "SyntaxError", "IndentationError", "TabError", "SystemError", "TypeError",
    "ValueError", "UnicodeError", "UnicodeDecodeError", "UnicodeEncodeError", "UnicodeTranslateError", "Warning",
    "BytesWarning", "DeprecationWarning", "EncodingWarning", "FutureWarning", "ImportWarning",
    "PendingDeprecationWarning", "ResourceWarning", "RuntimeWarning", "SyntaxWarning", "UnicodeWarning", "UserWarning",
];

pub fn is_builtin_exception(name: &str) -> bool {
    BUILTIN_EXCEPTIONS.contains(&name)
}

/// How a method is bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MethodKind {
    Instance,
    Static,
    Class,
    /// `@property` getter.
    Property,
    /// `@name.setter`.
    Setter,
}

#[derive(Clone, Debug)]
pub struct Method {
    pub name: String,
    pub func: FuncId,
    pub kind: MethodKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClassBase {
    /// A natively compiled class.
    Native(ClassId),
    /// A builtin exception class (`class MyError(ValueError)`).
    Exception(String),
}

/// A natively compiled class.
#[derive(Clone, Debug)]
pub struct ClassDef {
    pub id: ClassId,
    /// Python name (`Point`, or `geo.Point` for an imported unit).
    pub name: String,
    /// Generated Rust type name.
    pub rust: String,
    pub unit: usize,
    pub span: Span,
    pub base: Option<ClassBase>,
    pub methods: Vec<Method>,
    /// Instance attributes assigned through `self.<name> = ...` in its methods.
    pub fields: Vec<String>,
    /// Class attributes (stored as global cells named `Class.attr`).
    pub class_vars: Vec<String>,
    pub dataclass: bool,
}

impl Module {
    pub fn class(&self, c: ClassId) -> &ClassDef {
        &self.classes[c as usize]
    }

    /// Classes from `c` up its base chain.
    pub fn mro(&self, c: ClassId) -> Vec<ClassId> {
        let mut out = vec![c];
        let mut cur = c;
        while let Some(ClassBase::Native(b)) = &self.class(cur).base {
            if out.contains(b) {
                break;
            }
            out.push(*b);
            cur = *b;
        }
        out
    }

    /// Method lookup along the MRO.
    pub fn find_method(&self, c: ClassId, name: &str) -> Option<&Method> {
        self.mro(c).into_iter().find_map(|k| self.class(k).methods.iter().find(|m| m.name == name))
    }

    /// The class (along the MRO) defining class attribute `name`.
    pub fn find_class_var(&self, c: ClassId, name: &str) -> Option<ClassId> {
        self.mro(c).into_iter().find(|k| self.class(*k).class_vars.iter().any(|v| v == name))
    }

    pub fn is_subclass(&self, c: ClassId, base: ClassId) -> bool {
        self.mro(c).contains(&base)
    }

    /// The builtin exception class at the root of `c`'s bases, if any.
    pub fn exception_base(&self, c: ClassId) -> Option<String> {
        self.mro(c).into_iter().find_map(|k| match &self.class(k).base {
            Some(ClassBase::Exception(e)) => Some(e.clone()),
            _ => None,
        })
    }

    /// Instance attributes collected for `c` and its bases (bases first).
    pub fn declared_fields(&self, c: ClassId) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for k in self.mro(c).into_iter().rev() {
            for f in &self.class(k).fields {
                if !out.contains(f) {
                    out.push(f.clone());
                }
            }
        }
        out
    }
}

/// How arguments bind to a function's parameters.
#[derive(Clone, Debug, Default)]
pub struct CallSig {
    /// Positional-only, then positional-or-keyword, then keyword-only names.
    pub names: Vec<String>,
    pub n_posonly: usize,
    /// Number of parameters that can be passed positionally.
    pub n_pos: usize,
    /// Literal default per parameter (already lowered).
    pub defaults: Vec<Option<Expr>>,
    pub vararg: Option<String>,
    pub kwarg: Option<String>,
}

/// Why a call could not be bound (CPython's TypeError wording).
#[derive(Clone, Debug)]
pub enum BindError {
    TypeError(String),
    Unsupported(String),
}

fn builtin_call(name: &str, args: Vec<Expr>, kwargs: Vec<(String, Expr)>, sp: Span) -> Expr {
    Expr {
        kind: ExprKind::CallObj { func: Box::new(Expr { kind: ExprKind::PyBuiltin(name.into()), span: sp }), args, kwargs },
        span: sp,
    }
}

/// Place positional and keyword arguments, defaults, `*args` and `**kwargs`
/// into parameter order. Returns the arguments and their evaluation order
/// (empty when it is already parameter order).
pub fn bind_call(sig: &CallSig, fname: &str, args: Vec<Expr>, kwargs: Vec<(String, Expr)>, sp: Span) -> Result<(Vec<Expr>, Vec<usize>), BindError> {
    let te = |m: String| BindError::TypeError(m);
    let mut slots: Vec<Option<(Expr, usize)>> = vec![None; sig.names.len()];
    if kwargs.iter().any(|(k, _)| k == "**") {
        return Err(BindError::Unsupported(format!("`**mapping` in a call of the native function `{fname}`")));
    }
    let first_star = args.iter().position(|a| matches!(a.kind, ExprKind::Starred(_)));
    if let Some(fs) = first_star {
        if sig.vararg.is_none() || fs < sig.n_pos {
            return Err(BindError::Unsupported(format!(
                "`*iterable` argument to `{fname}` (a native function's fixed parameters need explicit arguments; `*xs` can only fill a `*args` parameter)"
            )));
        }
    }
    let npos = args.len();
    if npos > sig.n_pos && sig.vararg.is_none() {
        return Err(te(format!("{fname}() takes {} positional argument(s) but {npos} were given", sig.n_pos)));
    }
    let mut args = args;
    let extras: Vec<Expr> = if npos > sig.n_pos { args.split_off(sig.n_pos) } else { vec![] };
    for (i, a) in args.into_iter().enumerate() {
        slots[i] = Some((a, i));
    }
    let mut kw_extra: Vec<(String, Expr)> = Vec::new();
    let mut kw_extra_at = None;
    for (k, (kw, v)) in kwargs.into_iter().enumerate() {
        let Some(idx) = sig.names.iter().position(|n| *n == kw).filter(|&i| i >= sig.n_posonly) else {
            if sig.kwarg.is_some() {
                kw_extra_at.get_or_insert(npos + k);
                kw_extra.push((kw, v));
                continue;
            }
            return Err(te(format!("{fname}() got an unexpected keyword argument '{kw}'")));
        };
        if slots[idx].is_some() {
            return Err(te(format!("{fname}() got multiple values for argument '{kw}'")));
        }
        slots[idx] = Some((v, npos + k));
    }
    let mut out = Vec::new();
    let mut src = Vec::new();
    let mut missing = Vec::new();
    for (i, slot) in slots.into_iter().enumerate() {
        match slot {
            Some((e, at)) => {
                src.push((at, i));
                out.push(e);
            }
            None => match sig.defaults.get(i).cloned().flatten() {
                Some(d) => out.push(Expr { span: sp, ..d }),
                None => {
                    missing.push(format!("'{}'", sig.names[i]));
                    out.push(Expr { kind: ExprKind::NoneLit, span: sp });
                }
            },
        }
    }
    if !missing.is_empty() {
        return Err(te(format!("{fname}() missing {} required argument(s): {}", missing.len(), missing.join(", "))));
    }
    if sig.vararg.is_some() && !extras.iter().any(|x| matches!(x.kind, ExprKind::Starred(_))) {
        // A fixed-shape tuple: forwarding `f(*args)` can then expand it statically.
        src.push((sig.n_pos, out.len()));
        out.push(Expr { kind: ExprKind::Tuple(extras), span: sp });
    } else if sig.vararg.is_some() {
        let mut list: Option<Expr> = None;
        let mut plain = Vec::new();
        let concat = |l: Option<Expr>, part: Expr| match l {
            None => part,
            Some(l) => Expr { kind: ExprKind::BinOp { op: BinOp::Add, left: Box::new(l), right: Box::new(part) }, span: sp },
        };
        for x in extras {
            match x.kind {
                ExprKind::Starred(inner) => {
                    if !plain.is_empty() {
                        list = Some(concat(list, Expr { kind: ExprKind::List(std::mem::take(&mut plain)), span: sp }));
                    }
                    list = Some(concat(list, builtin_call("list", vec![*inner], vec![], sp)));
                }
                _ => plain.push(x),
            }
        }
        if !plain.is_empty() || list.is_none() {
            list = Some(concat(list, Expr { kind: ExprKind::List(plain), span: sp }));
        }
        src.push((sig.n_pos, out.len()));
        out.push(builtin_call("tuple", vec![list.unwrap()], vec![], sp));
    }
    if sig.kwarg.is_some() {
        src.push((kw_extra_at.unwrap_or(usize::MAX), out.len()));
        out.push(builtin_call("dict", vec![], kw_extra, sp));
    }
    src.sort();
    let order: Vec<usize> = src.iter().map(|&(_, i)| i).collect();
    let identity = order.windows(2).all(|w| w[0] < w[1]);
    Ok((out, if identity { vec![] } else { order }))
}
