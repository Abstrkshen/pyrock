//! Structured, scope-resolved HIR. Python evaluation order is preserved:
//! nothing here reorders sub-expressions.

use crate::{Span, Ty};

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
}

#[derive(Clone, Debug)]
pub struct ExternFn {
    pub name: String,
    pub params: Vec<ExtTy>,
    pub ret: ExtTy,
    /// Rust `Result<T, E>` return: errors raise `RuntimeError(str(e))`.
    pub fallible: bool,
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
}

impl ExtTy {
    pub fn ty(&self) -> Ty {
        match self {
            ExtTy::Int(_) => Ty::Int,
            ExtTy::F64 | ExtTy::F32 => Ty::Float,
            ExtTy::Bool => Ty::Bool,
            ExtTy::StrRef | ExtTy::OwnedString | ExtTy::CStr => Ty::Str,
            ExtTy::Unit => Ty::None,
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
        }
    }

    /// Whether a value of type `t` can be passed for this parameter.
    pub fn accepts(&self, t: Ty) -> bool {
        match self.ty() {
            _ if t == Ty::Obj => true, // converted at runtime, TypeError on mismatch
            Ty::Int => matches!(t, Ty::Int | Ty::Bool),
            Ty::Float => t.is_numeric(),
            Ty::Bool => t == Ty::Bool,
            Ty::Str => t == Ty::Str,
            Ty::None | Ty::Obj | Ty::C(_) => false,
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
}

impl Function {
    /// Rust-safe base name (`helpers.area` -> `helpers_area`).
    pub fn rust_base(&self) -> String {
        self.name.replace('.', "_")
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
    ForIter { var: String, iter: Expr, body: Vec<Stmt> },
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
    With { ctx: Expr, var: Option<String>, body: Vec<Stmt> },
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

#[derive(Clone, Debug)]
pub enum ExcType {
    /// A builtin exception class by name (`ValueError`, `OSError`, ...).
    Builtin(String),
    /// Any other expression: a Python exception class, matched with `isinstance`.
    Obj(Expr),
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
    /// `args` are in parameter order (keyword arguments and defaults already
    /// placed). `order`, when not empty, lists argument indices in Python's
    /// evaluation (source) order; defaults are not listed (they are literals).
    Call { callee: Callee, args: Vec<Expr>, order: Vec<usize> },
    BinOp { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    Unary { op: UnaryOp, operand: Box<Expr> },
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
