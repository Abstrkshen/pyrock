//! Frontend: parses Python files with CPython's own `ast` module through a
//! small versioned JSON protocol (see `dump_ast.py`), resolves imports, and
//! lowers everything into one scope-resolved HIR program.
//!
//! `import x` resolves, in order, next to the importing file:
//!   1. `x.py` — compiled natively as another unit (falls back to CPython in
//!      hybrid mode if it uses unsupported syntax);
//!   2. `x.rs` — Rust source linked into the binary;
//!   3. `x.c`  — C source compiled with the system C compiler;
//!   4. otherwise a CPython import at runtime (stdlib, site-packages, and
//!      C-extension packages such as numpy).

pub mod externs;

mod desugar;
use pyrock_ir::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

const DUMP_AST: &str = include_str!("dump_ast.py");
pub const PROTOCOL: u64 = 1;

pub struct ParseOutput {
    pub module: Option<Module>,
    pub diagnostics: Vec<Diagnostic>,
    pub python_version: String,
}

/// Load `path` and everything it imports natively.
pub fn parse_file(path: &str, python: &str, mode: Mode) -> Result<ParseOutput, String> {
    let entry = PathBuf::from(path);
    let root_dir = entry
        .parent()
        .map(|p| if p.as_os_str().is_empty() { Path::new(".") } else { p })
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| format!("{path}: {e}"))?;
    let mut ld = Loader {
        python: python.into(),
        mode,
        funcs: vec![],
        units: vec![],
        unit_funcs: vec![],
        unit_by_path: HashMap::new(),
        loading: HashSet::new(),
        externs: vec![],
        extern_by_path: HashMap::new(),
        slots: vec![],
        imports: vec![],
        builtins: HashSet::new(),
        python_version: String::new(),
        func_anns: HashMap::new(),
        func_sigs: HashMap::new(),
        unit_globals: vec![],
        classes: vec![],
        unit_classes: vec![],
        func_parent: HashMap::new(),
        func_captures: HashMap::new(),
        func_cells: HashMap::new(),
        unit_subs: vec![],
        unit_bindings: vec![],
        entry_dir: std::path::absolute(&entry).ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from(".")),
    };
    let (unit, diags) = ld.load_unit(&entry, "__main__", path);
    let Some(unit) = unit else {
        return Ok(ParseOutput { module: None, diagnostics: diags, python_version: ld.python_version });
    };
    let main = ld.units[unit].init;
    let mut funcs: Vec<Function> = ld.funcs.into_iter().map(|f| f.expect("function slot never filled")).collect();
    for f in &mut funcs {
        f.parent = ld.func_parent.get(&f.id).copied();
        f.captures = ld.func_captures.remove(&f.id).unwrap_or_default();
        f.cells = ld.func_cells.remove(&f.id).unwrap_or_default();
    }
    let mut globals: Vec<(usize, String)> = Vec::new();
    for (u, g) in ld.unit_globals.iter().enumerate() {
        let mut names: Vec<&String> = g.iter().collect();
        names.sort();
        globals.extend(names.into_iter().map(|n| (u, n.clone())));
    }
    let module = Module {
        file: path.to_string(),
        funcs,
        main,
        units: ld.units,
        externs: ld.externs,
        slots: ld.slots,
        root_dir: root_dir.display().to_string(),
        imports: ld.imports,
        globals,
        classes: ld.classes,
    };
    Ok(ParseOutput { module: Some(module), diagnostics: diags, python_version: ld.python_version })
}

struct Loader {
    python: String,
    mode: Mode,
    funcs: Vec<Option<Function>>,
    units: Vec<Unit>,
    unit_funcs: Vec<HashMap<String, FuncId>>,
    unit_by_path: HashMap<PathBuf, usize>,
    loading: HashSet<PathBuf>,
    externs: Vec<ExternModule>,
    extern_by_path: HashMap<PathBuf, usize>,
    slots: Vec<PySlot>,
    imports: Vec<ImportRecord>,
    builtins: HashSet<String>,
    python_version: String,
    /// Annotations per function: (parameters, return).
    func_anns: HashMap<FuncId, (Vec<Option<Ty>>, Option<Ty>)>,
    /// Call signatures of natively compiled functions.
    func_sigs: HashMap<FuncId, CallSig>,
    /// Per unit: module-level variables kept in global cells.
    unit_globals: Vec<HashSet<String>>,
    classes: Vec<ClassDef>,
    /// Per unit: class name -> class.
    unit_classes: Vec<HashMap<String, ClassId>>,
    /// Closure data per function: enclosing function, captured names, cell names.
    func_parent: HashMap<FuncId, FuncId>,
    func_captures: HashMap<FuncId, Vec<String>>,
    func_cells: HashMap<FuncId, Vec<String>>,
    /// Per unit: its submodules that have been imported (`pkg.sub` sets `sub` on `pkg`).
    unit_subs: Vec<HashMap<String, usize>>,
    /// Directory of the entry file: the root for package imports, as on `sys.path`.
    entry_dir: PathBuf,
    /// Per unit: its module-level bindings once its imports are resolved, so
    /// names it imported can be re-exported (`from .compat import urlparse`).
    unit_bindings: Vec<HashMap<String, Binding>>,
}

/// How arguments bind to a native function's parameters.
#[derive(Clone, Debug, Default)]
struct Sig {
    /// Positional-only, then positional-or-keyword, then keyword-only names.
    names: Vec<String>,
    n_posonly: usize,
    /// Number of parameters that can be passed positionally.
    n_pos: usize,
    /// Literal default per parameter (as AST JSON).
    defaults: Vec<Option<Value>>,
    /// `*args` / `**kwargs` parameter names (after `names` in `Function::params`).
    vararg: Option<String>,
    kwarg: Option<String>,
}

/// All parameter nodes of a `def`, in signature order, with the signature.
fn signature(a: &Value) -> (Vec<&Value>, Sig) {
    let posonly = arr(a, "posonlyargs");
    let args = arr(a, "args");
    let kwonly = arr(a, "kwonlyargs");
    let params: Vec<&Value> = posonly.iter().chain(args).chain(kwonly).collect();
    let n_pos = posonly.len() + args.len();
    let mut defaults: Vec<Option<Value>> = vec![None; params.len()];
    // `defaults` align with the last positional parameters.
    let pd = arr(a, "defaults");
    for (i, d) in pd.iter().enumerate() {
        defaults[n_pos - pd.len() + i] = Some(d.clone());
    }
    for (i, d) in arr(a, "kw_defaults").iter().enumerate() {
        if !d.is_null() {
            defaults[n_pos + i] = Some(d.clone());
        }
    }
    let names = params.iter().map(|p| sstr(p, "arg").to_string()).collect();
    let vararg = a["vararg"].get("arg").and_then(|v| v.as_str()).map(String::from);
    let kwarg = a["kwarg"].get("arg").and_then(|v| v.as_str()).map(String::from);
    (params, Sig { names, n_posonly: posonly.len(), n_pos, defaults, vararg, kwarg })
}

/// A literal default value as HIR (see `literal_default`).
fn lower_literal(v: &Value, unit: usize) -> Option<Expr> {
    let l = |i: usize| v["_loc"][i].as_u64().unwrap_or(0) as u32;
    let span = Span { line: l(0), col: l(1), end_line: l(2), end_col: l(3), unit: unit as u32 };
    let kind = match ty(v) {
        "Constant" => {
            let c = &v["value"];
            match c["kind"].as_str().unwrap_or("") {
                "none" => ExprKind::NoneLit,
                "bool" => ExprKind::Bool(c["value"].as_bool().unwrap_or(false)),
                "int" => ExprKind::Int(c["value"].as_str()?.parse().ok()?),
                "float" => ExprKind::Float(c["value"].as_str()?.parse().ok()?),
                "str" => ExprKind::Str(sstr(c, "value").to_string()),
                _ => return None,
            }
        }
        "UnaryOp" => {
            let inner = lower_literal(&v["operand"], unit)?;
            match (ty(&v["op"]), inner.kind) {
                ("USub", ExprKind::Int(i)) => ExprKind::Int(i.checked_neg()?),
                ("USub", ExprKind::Float(f)) => ExprKind::Float(-f),
                ("UAdd", k @ (ExprKind::Int(_) | ExprKind::Float(_))) => k,
                _ => return None,
            }
        }
        "Tuple" => ExprKind::Tuple(arr(v, "elts").iter().map(|e| lower_literal(e, unit)).collect::<Option<_>>()?),
        _ => return None,
    };
    Some(Expr { kind, span })
}

/// The call signature with lowered literal defaults.
fn call_sig(sig: &Sig, unit: usize) -> CallSig {
    CallSig {
        names: sig.names.clone(),
        n_posonly: sig.n_posonly,
        n_pos: sig.n_pos,
        defaults: sig.defaults.iter().map(|d| d.as_ref().and_then(|d| lower_literal(d, unit))).collect(),
        vararg: sig.vararg.clone(),
        kwarg: sig.kwarg.clone(),
    }
}

/// Defaults are evaluated once, at `def` time, in CPython. pyrock inlines
/// them at each call, which is only equivalent for immutable literals.
fn literal_default(v: &Value) -> bool {
    match ty(v) {
        "Constant" => v["value"]["kind"] != "other",
        "UnaryOp" => matches!(ty(&v["op"]), "USub" | "UAdd") && ty(&v["operand"]) == "Constant",
        "Tuple" => arr(v, "elts").iter().all(literal_default),
        _ => false,
    }
}

impl Loader {
    fn dump(&self, path: &Path) -> Result<Value, String> {
        let out = Command::new(&self.python)
            .arg("-I")
            .arg("-c")
            .arg(DUMP_AST)
            .arg(path)
            .output()
            .map_err(|e| format!("failed to run `{}` for parsing: {e}", self.python))?;
        if !out.status.success() {
            return Err(format!("frontend parser failed on {}:\n{}", path.display(), String::from_utf8_lossy(&out.stderr)));
        }
        let json: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("bad AST JSON: {e}"))?;
        if json["protocol"].as_u64() != Some(PROTOCOL) {
            return Err(format!("frontend protocol mismatch: expected {PROTOCOL}, got {}", json["protocol"]));
        }
        Ok(json)
    }

    /// Parse and lower one Python file. Returns the unit id (None if it
    /// could not be parsed at all) and the diagnostics for that file.
    fn load_unit(&mut self, path: &Path, name: &str, display: &str) -> (Option<usize>, Vec<Diagnostic>) {
        let uid = self.units.len();
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.units.push(Unit { name: name.into(), path: display.into(), init: usize::MAX });
        self.unit_funcs.push(HashMap::new());
        self.unit_globals.push(HashSet::new());
        self.unit_classes.push(HashMap::new());
        self.unit_subs.push(HashMap::new());
        self.unit_bindings.push(HashMap::new());
        self.unit_by_path.insert(canon.clone(), uid);
        let at = |line: u32, col: u32| Span { line, col, end_line: line, end_col: col, unit: uid as u32 };
        let json = match self.dump(path) {
            Ok(j) => j,
            Err(e) => return (None, vec![Diagnostic::error("P002", at(1, 1), e)]),
        };
        if let Some(err) = json.get("error") {
            let (l, c) = (err["line"].as_u64().unwrap_or(0) as u32, err["col"].as_u64().unwrap_or(0) as u32);
            let msg = format!("SyntaxError: {}", err["msg"].as_str().unwrap_or("?"));
            return (None, vec![Diagnostic::error("P001", at(l, c), msg)]);
        }
        if uid == 0 {
            self.python_version = json["python"].as_str().unwrap_or("").to_string();
            self.builtins = json["builtins"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect();
        }
        self.loading.insert(canon.clone());
        let dir = canon.parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut json = json;
        desugar::rewrite(&mut json["module"]);
        let mut lw = Lowerer::new(self, uid, dir);
        lw.lower_unit(&json["module"]);
        let diags = std::mem::take(&mut lw.diags);
        self.loading.remove(&canon);
        (Some(uid), diags)
    }

    fn alloc_func(&mut self) -> FuncId {
        self.funcs.push(None);
        self.funcs.len() - 1
    }
}

/// What a module-level name refers to.
#[derive(Clone, Debug)]
enum Binding {
    Func(FuncId),
    Unit(usize),
    Extern(usize),
    ExternFn(usize, usize),
    Slot(usize),
    Class(ClassId),
    /// A struct of a Rust/C module (`from shapes import Counter`).
    ExternStruct(usize, usize),
}

struct Lowerer<'l> {
    ld: &'l mut Loader,
    unit: usize,
    dir: PathBuf,
    func_ids: HashMap<String, (FuncId, Span)>,
    bindings: HashMap<String, Binding>,
    /// Import statements resolved in pass 1, keyed by their span.
    import_actions: HashMap<Span, Vec<ImportAction>>,
    module_vars: HashSet<String>,
    /// Module variables of this unit kept in global cells.
    globals: HashSet<String>,
    /// Names declared `global` in the function being lowered.
    fn_globals: HashSet<String>,
    /// Synthetic locals created while lowering the current function.
    extra_locals: Vec<String>,
    /// Counter for synthetic locals (`__u1`, ...).
    tmp: usize,
    /// The class whose method is being lowered, and the name of its `self` parameter.
    cur_class: Option<(ClassId, Option<String>)>,
    /// Classes of this unit: name -> (class, definition span).
    class_ids: HashMap<String, (ClassId, Span)>,
    /// Comprehension (and `except ... as`) scopes: source name -> renamed
    /// local, with the function that owns the renamed local.
    comp_scopes: Vec<HashMap<String, String>>,
    comp_owner: Vec<Option<FuncId>>,
    /// The function being lowered (None for a module body).
    cur_fid: Option<FuncId>,
    /// Enclosing functions of a nested function: (function, its locals).
    scopes: Vec<(Option<FuncId>, HashSet<String>)>,
    /// Names declared `nonlocal` in the function being lowered.
    fn_nonlocals: HashSet<String>,
    /// Top-level functions with decorators (their name is a module variable).
    decorated: HashSet<String>,
    /// `from mod import NAME` of module variables, per import statement:
    /// (binding, unit, name). The binding is a global of this unit, assigned
    /// once at the import (CPython copies the value at import time).
    var_imports: HashMap<Span, Vec<(String, usize, String)>>,
    locals: HashSet<String>,
    in_module: bool,
    /// Return annotation of the function being lowered.
    ret_ann: Option<Ty>,
    /// Import slots referenced by lowered code (unused `typing` imports are dropped).
    used_slots: HashSet<usize>,
    /// Non-literal defaults of top-level functions and methods: a hidden
    /// global per default, assigned where the `def` runs (CPython evaluates
    /// defaults once, at definition time).
    pending_defaults: HashMap<FuncId, Vec<(String, Value)>>,
    hidden_globals: HashSet<String>,
    /// Names a function binds by importing a native module inside its body:
    /// bindings for that function only, not local variables.
    native_imported: HashSet<String>,
    /// Imports of package modules that don't exist (`try: from ._speedups
    /// import f`): they raise ModuleNotFoundError where they run.
    missing_imports: HashMap<Span, String>,
    /// Lowering the body of this enum class: `Color("RED", 1)` there creates
    /// a member; elsewhere `Color(v)` looks one up.
    in_enum_body: Option<ClassId>,
    /// Local names of `abc.ABC` / `ABCMeta` / `abstractmethod` (by name) and of
    /// the `abc` module itself (key ""), from this module's imports.
    abc_names: HashMap<String, String>,
    diags: Vec<Diagnostic>,
}

/// Modules whose imports only matter to type checkers.
const ANNOTATION_ONLY_MODULES: &[&str] = &["typing", "typing_extensions", "__future__", "dataclasses", "abc", "enum", "functools"];

/// Map an annotation to a type pyrock can use. Anything else (generics,
/// unions, user classes) is accepted but does not constrain the value.
fn parse_ann(v: &Value) -> Option<Ty> {
    match ty(v) {
        "Name" => match sstr(v, "id") {
            "int" => Some(Ty::Int),
            "float" => Some(Ty::Float),
            "str" => Some(Ty::Str),
            "bool" => Some(Ty::Bool),
            "object" | "Any" => Some(Ty::Obj),
            _ => None,
        },
        "Attribute" if sstr(v, "attr") == "Any" => Some(Ty::Obj),
        "Constant" => match v["value"]["kind"].as_str() {
            Some("none") => Some(Ty::None),
            // String (forward-reference) annotation: "int"
            Some("str") => parse_ann(&serde_json::json!({"_t": "Name", "id": v["value"]["value"]})),
            _ => None,
        },
        _ => None,
    }
}

/// Annotations that convert values: the native scalar types.
fn converting(t: Option<Ty>) -> Option<Ty> {
    t.filter(|t| matches!(t, Ty::Int | Ty::Float | Ty::Str | Ty::Bool))
}

fn annotated(t: Option<Ty>, e: Expr) -> Expr {
    match converting(t) {
        Some(ty) => {
            let span = e.span;
            Expr { kind: ExprKind::Annotated { ty, value: Box::new(e) }, span }
        }
        None => e,
    }
}

fn ty(v: &Value) -> &str {
    v["_t"].as_str().unwrap_or("")
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v[k].as_array().map(|a| a.as_slice()).unwrap_or(&[])
}

fn sstr<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

/// Instance attributes assigned through `self_name.<attr> = ...` in a method body.
fn scan_self_fields(body: &[Value], self_name: &str, out: &mut Vec<String>) {
    fn target(t: &Value, self_name: &str, out: &mut Vec<String>) {
        match ty(t) {
            "Attribute" if ty(&t["value"]) == "Name" && sstr(&t["value"], "id") == self_name => {
                let a = sstr(t, "attr").to_string();
                if !out.contains(&a) {
                    out.push(a);
                }
            }
            "Tuple" | "List" => arr(t, "elts").iter().for_each(|e| target(e, self_name, out)),
            "Starred" => target(&t["value"], self_name, out),
            _ => {}
        }
    }
    for st in body {
        match ty(st) {
            "Assign" => arr(st, "targets").iter().for_each(|t| target(t, self_name, out)),
            "AugAssign" | "AnnAssign" => target(&st["target"], self_name, out),
            "For" | "AsyncFor" => target(&st["target"], self_name, out),
            "With" => arr(st, "items").iter().for_each(|i| target(&i["optional_vars"], self_name, out)),
            _ => {}
        }
        for k in ["body", "orelse", "finalbody"] {
            scan_self_fields(arr(st, k), self_name, out);
        }
        for h in arr(st, "handlers") {
            scan_self_fields(arr(h, "body"), self_name, out);
        }
    }
}

/// Struct types in an extern module's signatures refer to it by index.
fn fix_ext_index(m: &mut ExternModule, e: usize) {
    fn fix(t: &mut ExtTy, e: usize) {
        match t {
            ExtTy::Struct { ext, .. } => *ext = e,
            ExtTy::Vec(x) | ExtTy::Slice(x) | ExtTy::SliceMut(x, _) | ExtTy::Opt(x) | ExtTy::CArray(x, _, _) => fix(x, e),
            ExtTy::Tuple(xs) => xs.iter_mut().for_each(|x| fix(x, e)),
            ExtTy::Map(k, v, _) => {
                fix(k, e);
                fix(v, e);
            }
            ExtTy::Callback(ps, r, _) | ExtTy::CFn(ps, r) => {
                ps.iter_mut().for_each(|x| fix(x, e));
                fix(r, e);
            }
            _ => {}
        }
    }
    for f in &mut m.funcs {
        f.params.iter_mut().for_each(|p| fix(p, e));
        fix(&mut f.ret, e);
    }
    for st in &mut m.structs {
        st.fields.iter_mut().for_each(|(_, t)| fix(t, e));
    }
}

/// Number of real (non-default) captures of `fid`.
fn captures_len(m: &HashMap<FuncId, Vec<String>>, fid: FuncId, n_defaults: usize) -> usize {
    m.get(&fid).map(|v| v.len()).unwrap_or(0) - n_defaults
}

/// Every `Name` mentioned anywhere under `v`.
fn scan_names(v: &Value, out: &mut HashSet<String>) {
    match v {
        Value::Object(m) => {
            if m.get("_t").and_then(|t| t.as_str()) == Some("Name") {
                if let Some(id) = m.get("id").and_then(|i| i.as_str()) {
                    out.insert(id.to_string());
                }
            }
            for x in m.values() {
                scan_names(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| scan_names(x, out)),
        _ => {}
    }
}

/// Names declared `nonlocal` in a function body (not in nested functions).
fn nonlocal_decls(body: &[Value], out: &mut HashSet<String>) {
    for st in body {
        match ty(st) {
            "Nonlocal" => out.extend(arr(st, "names").iter().filter_map(|n| n.as_str().map(String::from))),
            "FunctionDef" | "ClassDef" | "AsyncFunctionDef" => {}
            _ => {
                for k in ["body", "orelse", "finalbody"] {
                    nonlocal_decls(arr(st, k), out);
                }
                for h in arr(st, "handlers") {
                    nonlocal_decls(arr(h, "body"), out);
                }
            }
        }
    }
}

/// Every name used inside `lambda` bodies under `v` (module-level lambdas
/// read module variables, which must then be global cells).
fn scan_lambda_names(v: &Value, out: &mut HashSet<String>) {
    match v {
        Value::Object(m) => {
            if m.get("_t").and_then(|t| t.as_str()) == Some("Lambda") {
                if let Some(b) = m.get("body") {
                    scan_names(b, out);
                }
            }
            // A generator expression is a hidden generator function too.
            if m.get("_t").and_then(|t| t.as_str()) == Some("GeneratorExp") {
                scan_names(v, out);
            }
            for x in m.values() {
                scan_lambda_names(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| scan_lambda_names(x, out)),
        _ => {}
    }
}

/// Names declared `global` in a function body (not in nested functions).
fn global_decls(body: &[Value], out: &mut HashSet<String>) {
    for st in body {
        match ty(st) {
            "Global" => out.extend(arr(st, "names").iter().filter_map(|n| n.as_str().map(String::from))),
            "FunctionDef" | "ClassDef" | "AsyncFunctionDef" => {}
            _ => {
                for k in ["body", "orelse", "finalbody"] {
                    global_decls(arr(st, k), out);
                }
                for h in arr(st, "handlers") {
                    global_decls(arr(h, "body"), out);
                }
            }
        }
    }
}

/// Collect names bound by assignment-like statements, in first-binding order.
/// Does not descend into nested function definitions.
fn collect_bindings(body: &[Value], out: &mut Vec<String>, top: bool) {
    collect_bindings_in(body, out, top, false)
}

/// Names bound by `:=` in a statement's own expressions (not in nested
/// functions, classes, lambdas or comprehensions, which have their own
/// scope; nested statements are visited by the caller).
fn walrus_targets(v: &Value, out: &mut Vec<String>) {
    fn go(v: &Value, out: &mut Vec<String>, top: bool) {
        match v {
            Value::Array(a) => a.iter().for_each(|x| go(x, out, false)),
            Value::Object(m) => {
                let t = m.get("_t").and_then(|t| t.as_str()).unwrap_or("");
                if !top
                    && matches!(
                        t,
                        // List/set/dict comprehensions are lowered inline, and
                        // `:=` in them binds this scope, as in CPython.
                        "FunctionDef" | "AsyncFunctionDef" | "ClassDef" | "Lambda" | "GeneratorExp"
                    )
                {
                    return;
                }
                if t == "NamedExpr" {
                    let n = sstr(&v["target"], "id");
                    if !out.iter().any(|x| x == n) {
                        out.push(n.to_string());
                    }
                }
                for (k, x) in m {
                    // Statement lists are handled by the caller.
                    if matches!(k.as_str(), "body" | "orelse" | "finalbody" | "handlers") && x.is_array() {
                        continue;
                    }
                    go(x, out, false);
                }
            }
            _ => {}
        }
    }
    go(v, out, true)
}

/// Names a module binds to `abc`'s members (`from abc import ABC as B` maps
/// `B` to `ABC`) and to the `abc` module itself (mapped to "").
fn abc_imports(body: &[Value]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for st in body {
        match ty(st) {
            "ImportFrom" if st["module"] == "abc" && st["level"].as_u64().unwrap_or(0) == 0 => {
                for a in arr(st, "names") {
                    out.insert(a["asname"].as_str().unwrap_or(sstr(a, "name")).to_string(), sstr(a, "name").to_string());
                }
            }
            "Import" => {
                for a in arr(st, "names") {
                    if sstr(a, "name") == "abc" {
                        out.insert(a["asname"].as_str().unwrap_or("abc").to_string(), String::new());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The names an `import` statement binds (`import a.b` binds `a`).
fn import_names(st: &Value) -> Vec<String> {
    arr(st, "names")
        .iter()
        .map(|a| match a["asname"].as_str() {
            Some(n) => n.to_string(),
            None if ty(st) == "Import" => sstr(a, "name").split('.').next().unwrap_or("").to_string(),
            None => sstr(a, "name").to_string(),
        })
        .filter(|n| n != "*")
        .collect()
}

/// `nested`: inside a block or function. A module's top-level imports bind
/// module-wide names (slots); nested ones assign a variable.
fn collect_bindings_in(body: &[Value], out: &mut Vec<String>, top: bool, nested: bool) {
    fn push(n: &str, out: &mut Vec<String>) {
        if !out.iter().any(|x| x == n) {
            out.push(n.to_string());
        }
    }
    /// Names bound by an assignment target (`a`, `a, (b, *c)`).
    fn target(t: &Value, out: &mut Vec<String>) {
        match ty(t) {
            "Name" => push(sstr(t, "id"), out),
            "Tuple" | "List" => arr(t, "elts").iter().for_each(|e| target(e, out)),
            "Starred" => target(&t["value"], out),
            _ => {}
        }
    }
    for st in body {
        walrus_targets(st, out);
        match ty(st) {
            "Assign" => {
                for t in arr(st, "targets") {
                    target(t, out);
                }
            }
            "AugAssign" | "AnnAssign" => {
                if ty(&st["target"]) == "Name" {
                    push(sstr(&st["target"], "id"), out);
                }
            }
            "For" => {
                target(&st["target"], out);
                collect_bindings_in(arr(st, "body"), out, top, true);
                collect_bindings_in(arr(st, "orelse"), out, top, true);
            }
            "If" | "While" => {
                collect_bindings_in(arr(st, "body"), out, top, true);
                collect_bindings_in(arr(st, "orelse"), out, top, true);
            }
            "Try" => {
                collect_bindings_in(arr(st, "body"), out, top, true);
                for h in arr(st, "handlers") {
                    if let Some(n) = h["name"].as_str() {
                        push(n, out);
                    }
                    collect_bindings_in(arr(h, "body"), out, top, true);
                }
                collect_bindings_in(arr(st, "orelse"), out, top, true);
                collect_bindings_in(arr(st, "finalbody"), out, top, true);
            }
            "With" => {
                for item in arr(st, "items") {
                    target(&item["optional_vars"], out);
                }
                collect_bindings_in(arr(st, "body"), out, top, true);
            }
            "FunctionDef" | "AsyncFunctionDef" if nested || !top => push(sstr(st, "name"), out),
            "Import" | "ImportFrom" if nested || !top => {
                for n in import_names(st) {
                    push(&n, out);
                }
            }
            _ => {}
        }
    }
}

impl<'l> Lowerer<'l> {
    fn new(ld: &'l mut Loader, unit: usize, dir: PathBuf) -> Self {
        Lowerer {
            ld,
            unit,
            dir,
            func_ids: HashMap::new(),
            bindings: HashMap::new(),
            import_actions: HashMap::new(),
            module_vars: HashSet::new(),
            globals: HashSet::new(),
            fn_globals: HashSet::new(),
            extra_locals: vec![],
            var_imports: HashMap::new(),
            tmp: 0,
            cur_class: None,
            class_ids: HashMap::new(),
            comp_scopes: vec![],
            comp_owner: vec![],
            cur_fid: None,
            scopes: vec![],
            fn_nonlocals: HashSet::new(),
            decorated: HashSet::new(),
            locals: HashSet::new(),
            in_module: true,
            ret_ann: None,
            used_slots: HashSet::new(),
            pending_defaults: HashMap::new(),
            hidden_globals: HashSet::new(),
            native_imported: HashSet::new(),
            missing_imports: HashMap::new(),
            in_enum_body: None,
            abc_names: HashMap::new(),
            diags: vec![],
        }
    }

    fn span(&self, v: &Value) -> Span {
        let l = |i: usize| v["_loc"][i].as_u64().unwrap_or(0) as u32;
        Span { line: l(0), col: l(1), end_line: l(2), end_col: l(3), unit: self.unit as u32 }
    }

    fn err(&mut self, code: &'static str, sp: Span, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(code, sp, msg));
    }

    fn unsupported(&mut self, sp: Span, what: &str, note: &str) {
        self.diags.push(Diagnostic::error("P100", sp, format!("{what} is not supported yet")).note(note.to_string()));
    }

    fn qual(&self, name: &str) -> String {
        if self.unit == 0 {
            name.to_string()
        } else {
            format!("{}.{name}", self.ld.units[self.unit].name)
        }
    }

    fn lower_unit(&mut self, root: &Value) {
        let body = arr(root, "body");

        // Pass 1a: discover top-level functions.
        for st in body {
            if ty(st) == "FunctionDef" {
                let name = sstr(st, "name").to_string();
                let sp = self.span(st);
                if self.func_ids.contains_key(&name) {
                    self.err("P141", sp, format!("function `{name}` is redefined; global rebinding is not compiled natively"));
                    continue;
                }
                let id = self.ld.alloc_func();
                let (params, sig) = signature(&st["args"]);
                let mut anns: Vec<Option<Ty>> = params.iter().map(|p| parse_ann(&p["annotation"])).collect();
                anns.extend(sig.vararg.iter().chain(&sig.kwarg).map(|_| None));
                self.ld.func_anns.insert(id, (anns, parse_ann(&st["returns"])));
                self.ld.func_sigs.insert(id, call_sig(&sig, self.unit));
                self.hoist_defaults(id, &name, &sig, sp);
                self.func_ids.insert(name.clone(), (id, sp));
                if arr(st, "decorator_list").is_empty() {
                    self.bindings.insert(name.clone(), Binding::Func(id));
                    self.ld.unit_funcs[self.unit].insert(name, id);
                } else {
                    // `@dec def f`: the name is a module variable holding dec(f).
                    self.decorated.insert(name);
                }
            }
        }
        // Pass 1a': discover classes (in order: bases must be defined earlier),
        // before imports, so a module importing this one back while it loads
        // (a package import cycle) already sees its classes.
        self.abc_names = abc_imports(body);
        for st in body {
            if ty(st) == "ClassDef" {
                self.declare_class(st);
            }
        }
        // Pass 1b: resolve top-level imports (they are visible inside
        // functions). Imports in blocks and functions are lowered in place
        // (`lower_nested_import`).
        for st in body {
            if matches!(ty(st), "Import" | "ImportFrom") {
                self.resolve_import(st);
            }
        }
        // Native imports inside module-level blocks (`if TYPE_CHECKING:`,
        // `try: from .speedups import f`): module-wide bindings like the
        // top-level ones; CPython imports there assign variables instead.
        fn block_imports<'v>(stmts: &'v [Value], out: &mut Vec<&'v Value>) {
            for st in stmts {
                match ty(st) {
                    "Import" | "ImportFrom" => out.push(st),
                    "If" | "Try" | "With" | "For" | "While" => {
                        for key in ["body", "orelse", "finalbody"] {
                            block_imports(arr(st, key), out);
                        }
                        for h in arr(st, "handlers") {
                            block_imports(arr(h, "body"), out);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut nested = Vec::new();
        for st in body.iter().filter(|st| !matches!(ty(st), "Import" | "ImportFrom" | "FunctionDef" | "AsyncFunctionDef" | "ClassDef")) {
            block_imports(std::slice::from_ref(st), &mut nested);
        }
        let mut block_bound: HashSet<String> = HashSet::new();
        for st in nested {
            if self.native_import_target(st) {
                self.resolve_import(st);
                block_bound.extend(import_names(st));
            }
        }
        self.ld.unit_bindings[self.unit] = self.bindings.clone();
        let mut module_locals = Vec::new();
        collect_bindings(body, &mut module_locals, true);
        // A name bound only by a native import in a block is a binding, not a
        // variable; one also assigned elsewhere stays a variable (and then
        // conflicts with the binding: P141).
        if !block_bound.is_empty() {
            fn strip_imports(v: &mut Value) {
                if let Value::Object(m) = v {
                    for key in ["body", "orelse", "finalbody"] {
                        if let Some(Value::Array(stmts)) = m.get_mut(key) {
                            stmts.retain(|st| !matches!(ty(st), "Import" | "ImportFrom"));
                            stmts.iter_mut().for_each(strip_imports);
                        }
                    }
                    if let Some(Value::Array(hs)) = m.get_mut("handlers") {
                        hs.iter_mut().for_each(strip_imports);
                    }
                }
            }
            let mut stripped: Vec<Value> = body.iter().filter(|st| !matches!(ty(st), "Import" | "ImportFrom")).cloned().collect();
            stripped.iter_mut().for_each(strip_imports);
            let mut others = Vec::new();
            collect_bindings(&stripped, &mut others, true);
            module_locals.retain(|n| !block_bound.contains(n) || others.contains(n));
        }
        for name in &module_locals {
            if self.bindings.contains_key(name) && !self.decorated.contains(name) {
                let sp = self.func_ids.get(name).map(|x| x.1).unwrap_or_default();
                self.err("P141", sp, format!("module-level name `{name}` rebinds a function or import; global rebinding is not compiled natively"));
            }
        }
        self.module_vars = module_locals.iter().cloned().collect();
        // Module variables that functions read (or declare `global`) live in
        // global cells. All variables of an imported unit do, so importers can
        // read `mod.NAME`.
        let mut g: HashSet<String> = HashSet::new();
        let methods: Vec<&Value> = body
            .iter()
            .filter(|st| ty(st) == "ClassDef")
            .flat_map(|c| arr(c, "body").iter().filter(|m| ty(m) == "FunctionDef"))
            .collect();
        // Functions defined inside module-level blocks (`try: from _c import f`
        // / `except ImportError: def f(...)`) read module variables too.
        fn block_defs<'v>(stmts: &'v [Value], out: &mut Vec<&'v Value>) {
            for st in stmts {
                match ty(st) {
                    "FunctionDef" | "AsyncFunctionDef" => out.push(st),
                    "If" | "While" | "For" | "With" | "Try" => {
                        for key in ["body", "orelse", "finalbody"] {
                            block_defs(arr(st, key), out);
                        }
                        for h in arr(st, "handlers") {
                            block_defs(arr(h, "body"), out);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut nested_defs = Vec::new();
        for st in body.iter().filter(|st| !matches!(ty(st), "FunctionDef" | "AsyncFunctionDef" | "ClassDef")) {
            block_defs(std::slice::from_ref(st), &mut nested_defs);
        }
        for st in body.iter().filter(|st| ty(st) == "FunctionDef").chain(methods).chain(nested_defs) {
            let sig = signature(&st["args"]).1;
            let mut fl = sig.names;
            fl.extend(sig.vararg);
            fl.extend(sig.kwarg);
            collect_bindings(arr(st, "body"), &mut fl, false);
            let mut decl = HashSet::new();
            global_decls(arr(st, "body"), &mut decl);
            let mut used = HashSet::new();
            for s in arr(st, "body") {
                scan_names(s, &mut used);
            }
            for n in used {
                if self.module_vars.contains(&n) && (!fl.contains(&n) || decl.contains(&n)) {
                    g.insert(n);
                }
            }
            g.extend(decl.into_iter().filter(|n| !self.bindings.contains_key(n)));
        }
        for v in self.var_imports.values() {
            g.extend(v.iter().map(|(b, _, _)| b.clone()));
        }
        g.extend(self.decorated.iter().cloned());
        g.extend(self.hidden_globals.iter().cloned());
        let mut lam = HashSet::new();
        for st in body.iter().filter(|st| ty(st) != "FunctionDef") {
            scan_lambda_names(st, &mut lam);
        }
        g.extend(lam.into_iter().filter(|n| self.module_vars.contains(n)));
        if self.unit != 0 {
            g.extend(self.module_vars.iter().cloned());
        }
        // Class attributes are global cells named `Class.attr`.
        for (cname, (cid, _)) in &self.class_ids {
            for v in &self.ld.classes[*cid as usize].class_vars {
                g.insert(format!("{cname}.{v}"));
            }
        }
        self.module_vars.extend(g.iter().cloned());
        self.globals = g.clone();
        self.ld.unit_globals[self.unit] = g;

        // Pass 2: lower function bodies, then the module body.
        let mut main_body = Vec::new();
        self.locals = self.module_vars.difference(&self.globals).cloned().collect();
        self.in_module = true;
        for st in body {
            if ty(st) == "FunctionDef" {
                let name = sstr(st, "name");
                let Some(&(id, sp)) = self.func_ids.get(name) else { continue };
                if sp != self.span(st) {
                    continue; // duplicate, already reported
                }
                let saved = (std::mem::take(&mut self.locals), self.in_module, std::mem::take(&mut self.extra_locals), self.cur_fid);
                let f = self.lower_function(id, st);
                (self.locals, self.in_module, self.extra_locals, self.cur_fid) = saved;
                self.ld.funcs[id] = Some(f);
                main_body.extend(self.default_assigns(id));
                if self.decorated.contains(name) {
                    let fsp = self.span(st);
                    let mut value = Expr { kind: ExprKind::MakeClosure { func: id, captures: vec![], defaults: vec![] }, span: fsp };
                    for d in arr(st, "decorator_list").iter().rev() {
                        let Some(dec) = self.lower_expr(d) else { continue };
                        value = Expr { kind: ExprKind::CallObj { func: Box::new(dec), args: vec![value], kwargs: vec![] }, span: fsp };
                    }
                    main_body.push(Stmt { kind: StmtKind::AssignGlobal { unit: self.unit, name: name.to_string(), value }, span: fsp });
                }
            } else if ty(st) == "ClassDef" {
                let saved = (std::mem::take(&mut self.locals), self.in_module, std::mem::take(&mut self.extra_locals), self.cur_fid);
                let stmts = self.lower_class(st);
                (self.locals, self.in_module, self.extra_locals, self.cur_fid) = saved;
                main_body.extend(stmts);
            } else if let Some(s) = self.lower_stmt(st) {
                main_body.push(s);
            }
        }
        // `from typing import Optional` is only for type checkers: drop it
        // unless lowered code actually uses the imported name.
        let slots = &self.ld.slots;
        let used = &self.used_slots;
        let keep = |a: &ImportAction| match a {
            ImportAction::Py(s) => {
                let m = match &slots[*s].kind {
                    SlotKind::Import { module, .. } | SlotKind::From { module, .. } => module,
                };
                used.contains(s) || !ANNOTATION_ONLY_MODULES.contains(&m.split('.').next().unwrap_or(""))
            }
            _ => true,
        };
        main_body.retain_mut(|st| match &mut st.kind {
            StmtKind::Import(a) => keep(a),
            StmtKind::If { body, .. } => {
                body.retain(|s| !matches!(&s.kind, StmtKind::Import(a) if !keep(a)));
                true
            }
            _ => true,
        });
        let init = self.ld.alloc_func();
        let name = if self.unit == 0 { "<module>".to_string() } else { format!("{}.<module>", self.ld.units[self.unit].name) };
        let mut module_locals: Vec<String> = module_locals.into_iter().filter(|n| !self.globals.contains(n)).collect();
        module_locals.extend(std::mem::take(&mut self.extra_locals));
        self.ld.funcs[init] = Some(Function {
            id: init,
            name,
            params: vec![],
            locals: module_locals,
            body: main_body,
            span: Span { line: 1, col: 1, end_line: 1, end_col: 1, unit: self.unit as u32 },
            is_module: true,
            unit: self.unit,
            param_ann: vec![],
            ret_ann: None,
            class: None,
            sig: CallSig::default(),
            parent: None,
            captures: vec![],
            cells: vec![],
            is_async: false,
            is_generator: false,
        });
        self.ld.units[self.unit].init = init;
    }

    fn record(&mut self, name: &str, kind: &'static str, path: Option<&Path>, note: Option<String>) {
        if self.ld.imports.iter().any(|r| r.name == name && r.kind == kind) {
            return;
        }
        self.ld.imports.push(ImportRecord { name: name.into(), kind, path: path.map(|p| p.display().to_string()), note });
    }

    /// Resolve `name` next to the importing file. Returns a binding for a
    /// native unit or extern, or None for a CPython import.
    fn resolve_local(&mut self, name: &str, sp: Span) -> Option<Binding> {
        if name.contains('.') {
            return None;
        }
        let py = self.dir.join(format!("{name}.py"));
        if py.exists() {
            let canon = py.canonicalize().unwrap_or(py.clone());
            if let Some(&u) = self.ld.unit_by_path.get(&canon) {
                if self.ld.loading.contains(&canon) {
                    self.err("P145", sp, format!("circular import of `{name}` is not supported natively"));
                }
                return Some(Binding::Unit(u));
            }
            let display = py.display().to_string();
            let (uid, diags) = self.ld.load_unit(&py, name, &display);
            let failed = uid.is_none() || diags.iter().any(|d| d.severity == Severity::Error);
            if failed && self.ld.mode == Mode::Hybrid {
                let first = diags.iter().find(|d| d.severity == Severity::Error).map(|d| d.render(&display)).unwrap_or_default();
                self.diags.push(
                    Diagnostic::warning("P220", sp, format!("`{name}.py` cannot be compiled natively; importing it through CPython"))
                        .note(format!("first blocker: {first}")),
                );
                self.record(name, "python (CPython fallback)", Some(&py), Some(first));
                return None;
            }
            self.diags.extend(diags);
            self.record(name, "python (native)", Some(&py), None);
            return uid.map(Binding::Unit);
        }
        for (ext, lang) in [("rs", Lang::Rust), ("c", Lang::C)] {
            let p = self.dir.join(format!("{name}.{ext}"));
            if !p.exists() {
                continue;
            }
            let canon = p.canonicalize().unwrap_or(p.clone());
            if let Some(&e) = self.ld.extern_by_path.get(&canon) {
                return Some(Binding::Extern(e));
            }
            let loaded = match lang {
                Lang::Rust => externs::load_rust(name, &p),
                Lang::C => externs::load_c(name, &p),
            };
            match loaded {
                Ok(m) => {
                    let note = format!(
                        "{} function(s){}",
                        m.funcs.len(),
                        if m.skipped.is_empty() { String::new() } else { format!(", {} skipped", m.skipped.len()) }
                    );
                    let e = self.ld.externs.len();
                    let mut m = m;
                    fix_ext_index(&mut m, e);
                    self.ld.externs.push(m);
                    self.ld.extern_by_path.insert(canon, e);
                    self.record(name, if lang == Lang::Rust { "rust" } else { "c" }, Some(&p), Some(note));
                    return Some(Binding::Extern(e));
                }
                Err(msg) => {
                    self.err("P003", sp, msg);
                    return Some(Binding::Extern(usize::MAX));
                }
            }
        }
        None
    }

    fn add_slot(&mut self, binding: &str, kind: SlotKind) -> usize {
        self.ld.slots.push(PySlot { binding: binding.into(), kind });
        self.ld.slots.len() - 1
    }

    fn resolve_import(&mut self, st: &Value) {
        let sp = self.span(st);
        let mut actions = Vec::new();
        if ty(st) == "Import" {
            for alias in arr(st, "names") {
                let module = sstr(alias, "name").to_string();
                let asname = alias["asname"].as_str().map(String::from);
                let bind = asname.clone().unwrap_or_else(|| module.split('.').next().unwrap_or("").to_string());
                // A package next to the program (`import app.db` with
                // `app/__init__.py`): every package on the path runs, in order;
                // the name binds the top package (or, with `as`, the module).
                let parts: Vec<String> = module.split('.').map(String::from).collect();
                if module.contains('.') || self.is_package(&parts[0]) {
                    if let Some(chain) = self.resolve_chain(&parts, 0, sp) {
                        for &u in &chain {
                            actions.push(ImportAction::Unit(self.ld.units[u].init));
                        }
                        let b = if asname.is_some() { *chain.last().unwrap() } else { chain[0] };
                        self.bindings.insert(bind, Binding::Unit(b));
                        continue;
                    }
                }
                match self.resolve_local(&module, sp) {
                    Some(b) => {
                        if let Binding::Unit(u) = b {
                            actions.push(ImportAction::Unit(self.ld.units[u].init));
                        }
                        self.bindings.insert(bind, b);
                    }
                    None => {
                        let slot = self.add_slot(&bind, SlotKind::Import { module: module.clone(), bind_top: asname.is_none() });
                        self.record(&module, "cpython", None, None);
                        actions.push(ImportAction::Py(slot));
                        self.bindings.insert(bind, Binding::Slot(slot));
                    }
                }
            }
        } else {
            let level = st["level"].as_u64().unwrap_or(0) as usize;
            let module = st["module"].as_str().unwrap_or("").to_string();
            let parts: Vec<String> = if module.is_empty() { vec![] } else { module.split('.').map(String::from).collect() };
            let target = if level > 0 || module.contains('.') || parts.first().is_some_and(|p| self.is_package(p)) {
                match self.resolve_chain(&parts, level, sp) {
                    Some(chain) => {
                        for &u in &chain {
                            actions.push(ImportAction::Unit(self.ld.units[u].init));
                        }
                        Some(Binding::Unit(*chain.last().unwrap()))
                    }
                    None if level > 0 && self.relative_missing(&parts, level) => {
                        // Raises at runtime, as in CPython (often inside `try`).
                        let pkg = self.own_package().map(|p| p.1).unwrap_or_default();
                        let mut base = pkg;
                        for _ in 1..level {
                            base = base.rsplit_once('.').map(|x| x.0.to_string()).unwrap_or_default();
                        }
                        let full = if parts.is_empty() { base } else { format!("{base}.{}", parts.join(".")) };
                        self.missing_imports.insert(sp, full);
                        self.import_actions.insert(sp, vec![]);
                        return;
                    }
                    None if level > 0 => {
                        self.unsupported(sp, "relative import of a module that isn't compiled natively", "the package's modules must be next to each other as .py files");
                        return;
                    }
                    None => None,
                }
            } else {
                let t = self.resolve_local(&module, sp);
                if let Some(Binding::Unit(u)) = t {
                    actions.push(ImportAction::Unit(self.ld.units[u].init));
                }
                t
            };
            let module = if module.is_empty() { ".".repeat(level) } else { module };
            for alias in arr(st, "names") {
                let attr = sstr(alias, "name").to_string();
                let bind = alias["asname"].as_str().unwrap_or(&attr).to_string();
                if attr == "*" {
                    self.unsupported(sp, "`from ... import *`", "import the names you use explicitly");
                    continue;
                }
                match target {
                    Some(Binding::Unit(u)) => match self.ld.unit_funcs[u].get(&attr).copied().as_ref() {
                        Some(&fid) => {
                            self.bindings.insert(bind, Binding::Func(fid));
                        }
                        None if self.ld.unit_classes[u].contains_key(&attr) => {
                            let c = self.ld.unit_classes[u][&attr];
                            self.bindings.insert(bind, Binding::Class(c));
                        }
                        None if self.ld.unit_globals[u].contains(&attr) => {
                            self.var_imports.entry(sp).or_default().push((bind, u, attr.clone()));
                        }
                        // A name the module itself imported (a re-export).
                        None if self.ld.unit_bindings[u].contains_key(&attr) => {
                            let b = self.ld.unit_bindings[u][&attr].clone();
                            if let Binding::Slot(s) = b {
                                self.used_slots.insert(s);
                            }
                            self.bindings.insert(bind, b);
                        }
                        // `from pkg import sub`: a submodule of the package.
                        None if self.submodule(u, &attr, sp).is_some() => {
                            let s = self.submodule(u, &attr, sp).unwrap();
                            actions.push(ImportAction::Unit(self.ld.units[s].init));
                            self.bindings.insert(bind, Binding::Unit(s));
                        }
                        None => self.err("P140", sp, format!("`{module}` has no function, class or module-level variable `{attr}`")),
                    },
                    Some(Binding::Extern(e)) if e != usize::MAX => match self.ld.externs[e].funcs.iter().position(|f| f.name == attr) {
                        Some(i) => {
                            self.bindings.insert(bind, Binding::ExternFn(e, i));
                        }
                        None => match self.ld.externs[e].structs.iter().position(|st| st.name == attr) {
                            Some(si) => {
                                self.bindings.insert(bind, Binding::ExternStruct(e, si));
                            }
                            None => self.extern_missing(e, &attr, sp),
                        },
                    },
                    Some(_) => {}
                    None => {
                        let slot = self.add_slot(&bind, SlotKind::From { module: module.clone(), attr: attr.clone() });
                        actions.push(ImportAction::Py(slot));
                        self.bindings.insert(bind, Binding::Slot(slot));
                    }
                }
            }
            if target.is_none() {
                self.record(&module, "cpython", None, None);
            }
        }
        // A unit still loading (circular import, already reported) has no init yet.
        actions.retain(|a| !matches!(a, ImportAction::Unit(usize::MAX)));
        self.import_actions.insert(sp, actions);
    }

    /// An import inside a function or a block (`try: import x`, a lazy
    /// import in a function): a CPython import that assigns the bound names as
    /// variables (Python objects), as CPython does. Natively compiled modules
    /// (`.py`/`.rs`/`.c` next to the file) must be imported at module level,
    /// where their functions are bound for direct calls.
    fn lower_nested_import(&mut self, st: &Value) -> Option<Stmt> {
        let sp = self.span(st);
        // A natively compiled module (or package) imported inside a function:
        // resolved like a top-level import; the names it binds are bindings of
        // this function (calls stay direct), not local variables.
        if self.native_import_target(st) && self.cur_fid.is_some() && (ty(st) == "ImportFrom" || arr(st, "names").len() == 1) {
            self.resolve_import(st);
            if !self.import_actions.contains_key(&sp) {
                return None;
            }
            let var_names: HashSet<String> = self.var_imports.get(&sp).map(|v| v.iter().map(|x| x.0.clone()).collect()).unwrap_or_default();
            for n in import_names(st) {
                if !var_names.contains(&n) && self.bindings.contains_key(&n) {
                    self.locals.remove(&n);
                    self.native_imported.insert(n);
                }
            }
            return self.lower_stmt(st);
        }
        let mut stmts = Vec::new();
        let native = |me: &mut Self, module: &str| -> bool {
            if module.contains('.') || !["py", "rs", "c"].iter().any(|ext| me.dir.join(format!("{module}.{ext}")).exists()) {
                return false;
            }
            me.unsupported(
                sp,
                &format!("import of the natively compiled module `{module}` inside a function or block"),
                "import it at module level",
            );
            true
        };
        if ty(st) == "Import" {
            for alias in arr(st, "names") {
                let module = sstr(alias, "name").to_string();
                if native(self, &module) {
                    return None;
                }
                let asname = alias["asname"].as_str().map(String::from);
                let bind = asname.clone().unwrap_or_else(|| module.split('.').next().unwrap_or("").to_string());
                let slot = self.add_slot(&bind, SlotKind::Import { module: module.clone(), bind_top: asname.is_none() });
                self.record(&module, "cpython", None, None);
                stmts.push(Stmt { kind: StmtKind::Import(ImportAction::Py(slot)), span: sp });
                self.used_slots.insert(slot);
                let value = Expr { kind: ExprKind::Slot(slot), span: sp };
                stmts.push(Stmt { kind: self.bind_target(bind, value), span: sp });
            }
        } else {
            if st["level"].as_u64().unwrap_or(0) > 0 {
                self.unsupported(sp, "relative import", "use an absolute import next to the entry file");
                return None;
            }
            let module = sstr(st, "module").to_string();
            if native(self, &module) {
                return None;
            }
            for alias in arr(st, "names") {
                let attr = sstr(alias, "name").to_string();
                if attr == "*" {
                    self.unsupported(sp, "`from ... import *`", "import the names you use explicitly");
                    return None;
                }
                let bind = alias["asname"].as_str().unwrap_or(&attr).to_string();
                let slot = self.add_slot(&bind, SlotKind::From { module: module.clone(), attr: attr.clone() });
                stmts.push(Stmt { kind: StmtKind::Import(ImportAction::Py(slot)), span: sp });
                self.used_slots.insert(slot);
                let value = Expr { kind: ExprKind::Slot(slot), span: sp };
                stmts.push(Stmt { kind: self.bind_target(bind, value), span: sp });
            }
            self.record(&module, "cpython", None, None);
        }
        match stmts.len() {
            0 => None,
            1 => stmts.into_iter().next(),
            _ => Some(Stmt { kind: StmtKind::If { test: Expr { kind: ExprKind::Bool(true), span: sp }, body: stmts, orelse: vec![] }, span: sp }),
        }
    }

    /// Non-literal defaults of a top-level function or method (`qual` is its
    /// qualified name): calls read a hidden global, which `default_assigns`
    /// assigns where the `def` runs.
    fn hoist_defaults(&mut self, id: FuncId, qual: &str, sig: &Sig, sp: Span) {
        for (i, (n, d)) in sig.names.iter().zip(&sig.defaults).enumerate() {
            let Some(d) = d else { continue };
            if literal_default(d) {
                continue;
            }
            let g = format!("{qual}.<default {n}>");
            if let Some(cs) = self.ld.func_sigs.get_mut(&id) {
                cs.defaults[i] = Some(Expr { kind: ExprKind::Global { unit: self.unit, name: g.clone() }, span: sp });
            }
            self.hidden_globals.insert(g.clone());
            self.pending_defaults.entry(id).or_default().push((g, d.clone()));
        }
    }

    /// The assignments of `id`'s hoisted defaults, lowered in the current
    /// (module) scope, in parameter order.
    fn default_assigns(&mut self, id: FuncId) -> Vec<Stmt> {
        let mut out = Vec::new();
        for (g, d) in self.pending_defaults.remove(&id).unwrap_or_default() {
            let dsp = self.span(&d);
            if let Some(value) = self.lower_expr(&d) {
                out.push(Stmt { kind: StmtKind::AssignGlobal { unit: self.unit, name: g, value }, span: dsp });
            }
        }
        out
    }

    /// A container (or optional) annotation on a variable: `list[str]`,
    /// `dict[str, Agent]`, `set[int]`, `tuple[int, ...]`, `Optional[T]` /
    /// `T | None`, with scalar, class or container arguments.
    fn container_ann(&self, v: &Value) -> Option<Ty> {
        let elem = |me: &Self, v: &Value| -> Option<Ty> {
            if let Some(t) = parse_ann(v) {
                return Some(t);
            }
            if ty(v) == "Name" {
                if let Some(&(cid, _)) = me.class_ids.get(sstr(v, "id")) {
                    return Some(Ty::class(cid, &me.ld.classes[cid as usize].rust));
                }
            }
            me.container_ann(v)
        };
        match ty(v) {
            "Subscript" => {
                let base = match ty(&v["value"]) {
                    "Name" => sstr(&v["value"], "id"),
                    "Attribute" => sstr(&v["value"], "attr"),
                    _ => return None,
                };
                let args: Vec<&Value> = if ty(&v["slice"]) == "Tuple" { arr(&v["slice"], "elts").iter().collect() } else { vec![&v["slice"]] };
                match (base, args.as_slice()) {
                    ("list" | "List", [a]) => Some(Ty::list(elem(self, a)?)),
                    ("set" | "Set", [a]) => Some(Ty::set(elem(self, a)?)),
                    ("dict" | "Dict", [k, val]) => Some(Ty::dict(elem(self, k)?, elem(self, val)?)),
                    ("tuple" | "Tuple", [a, e]) if ty(e) == "Constant" && e["value"]["type"] == "ellipsis" => Some(Ty::tuple_var(elem(self, a)?)),
                    ("Optional", [a]) => Some(Ty::opt(elem(self, a)?)),
                    _ => None,
                }
            }
            "BinOp" if ty(&v["op"]) == "BitOr" => {
                let (l, r) = (&v["left"], &v["right"]);
                let is_none = |x: &Value| ty(x) == "Constant" && x["value"]["kind"] == "none";
                match (is_none(l), is_none(r)) {
                    (false, true) => Some(Ty::opt(elem(self, l)?)),
                    (true, false) => Some(Ty::opt(elem(self, r)?)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// A relative import whose module file doesn't exist in the package.
    fn relative_missing(&self, parts: &[String], level: usize) -> bool {
        let Some((mut dir, _)) = self.own_package() else { return false };
        for _ in 1..level {
            match dir.parent() {
                Some(p) => dir = p.to_path_buf(),
                None => return false,
            }
        }
        for (i, part) in parts.iter().enumerate() {
            let pkg = dir.join(part);
            if pkg.join("__init__.py").exists() {
                dir = pkg;
                continue;
            }
            return !(i + 1 == parts.len() && dir.join(format!("{part}.py")).exists());
        }
        false
    }

    /// Whether an import statement names a natively compiled module: relative,
    /// a package next to the program, or a `.py`/`.rs`/`.c` file next to it.
    fn native_import_target(&self, st: &Value) -> bool {
        let level = st["level"].as_u64().unwrap_or(0) as usize;
        let modname = if ty(st) == "ImportFrom" { st["module"].as_str().unwrap_or("").to_string() } else { arr(st, "names").first().map(|a| sstr(a, "name").to_string()).unwrap_or_default() };
        let first = modname.split('.').next().unwrap_or("").to_string();
        level > 0 || self.is_package(&first) || (!modname.contains('.') && ["py", "rs", "c"].iter().any(|ext| self.dir.join(format!("{first}.{ext}")).exists()))
    }

    /// `name` from the `abc` module: `ABC` / `abc.ABC` (with `from abc import
    /// ABC` / `import abc`).
    fn is_abc_name(&self, v: &Value, name: &str) -> bool {
        match ty(v) {
            "Name" => self.abc_names.get(sstr(v, "id")).is_some_and(|n| n == name),
            "Attribute" => sstr(v, "attr") == name && ty(&v["value"]) == "Name" && self.abc_names.get(sstr(&v["value"], "id")).is_some_and(|n| n.is_empty()),
            _ => false,
        }
    }

    /// The enum class a `Name` / `mod.Name` refers to.
    fn enum_class_of(&self, v: &Value) -> Option<ClassId> {
        let c = match ty(v) {
            "Name" if !self.locals.contains(sstr(v, "id")) => match self.bindings.get(sstr(v, "id")) {
                Some(Binding::Class(c)) => *c,
                _ => return None,
            },
            "Attribute" => match self.module_binding(&v["value"]) {
                Some(Binding::Unit(u)) => *self.ld.unit_classes[u].get(sstr(v, "attr"))?,
                _ => return None,
            },
            _ => return None,
        };
        self.ld.classes[c as usize].enum_kind.is_some().then_some(c)
    }

    /// An enum class used as an iterable: its member list.
    fn enum_members_json(&self, v: &Value) -> Value {
        if self.enum_class_of(v).is_some() {
            serde_json::json!({"_t": "Attribute", "value": v, "attr": "_pyrock_members_", "ctx": {"_t": "Load"}, "_loc": v["_loc"]})
        } else {
            v.clone()
        }
    }

    /// A hidden name bound to what module `u` binds as `attr` (an import it
    /// re-exports, or a submodule), as a `Name` node to lower in its place.
    fn reexport_name(&mut self, u: usize, attr: &str, loc: &Value) -> Value {
        let hidden = format!("__reexport_{u}_{attr}");
        let b = self.ld.unit_subs[u].get(attr).map(|&s| Binding::Unit(s)).unwrap_or_else(|| self.ld.unit_bindings[u][attr].clone());
        if let Binding::Slot(s) = b {
            self.used_slots.insert(s);
        }
        self.bindings.insert(hidden.clone(), b);
        serde_json::json!({"_t": "Name", "id": hidden, "ctx": {"_t": "Load"}, "_loc": loc})
    }

    /// Directories a top-level (absolute) import is looked up in: next to the
    /// importing file, then the entry file's directory (`sys.path[0]`).
    fn import_bases(&self) -> Vec<PathBuf> {
        let mut v = vec![self.dir.clone()];
        let e = self.ld.entry_dir.clone();
        if e.canonicalize().ok() != self.dir.canonicalize().ok() {
            v.push(e);
        }
        v
    }

    /// Whether `name` is a package directory (with `__init__.py`) on the import path.
    fn is_package(&self, name: &str) -> bool {
        self.import_bases().iter().any(|b| b.join(name).join("__init__.py").exists())
    }

    /// The package this unit belongs to (for relative imports): its directory
    /// and dotted name. `None` for a top-level module or the entry file.
    fn own_package(&self) -> Option<(PathBuf, String)> {
        let u = &self.ld.units[self.unit];
        let is_init = u.path.ends_with("__init__.py");
        let name = if is_init { Some(u.name.clone()) } else { u.name.rsplit_once('.').map(|(a, _)| a.to_string()) };
        name.map(|n| (self.dir.clone(), n))
    }

    /// The unit of a package's or module's file, loading it if needed
    /// (`None`: imported through CPython instead, P220).
    fn module_unit(&mut self, path: &Path, qual: &str, sp: Span) -> Option<usize> {
        let canon = path.canonicalize().unwrap_or(path.to_path_buf());
        if let Some(&u) = self.ld.unit_by_path.get(&canon) {
            // Still loading: a package whose `__init__` imports its own
            // submodules is already (partly) initialised, as in CPython.
            return Some(u);
        }
        let display = path.display().to_string();
        let (uid, diags) = self.ld.load_unit(path, qual, &display);
        let failed = uid.is_none() || diags.iter().any(|d| d.severity == Severity::Error);
        if failed && self.ld.mode == Mode::Hybrid {
            let first = diags.iter().find(|d| d.severity == Severity::Error).map(|d| d.render(&display)).unwrap_or_default();
            self.diags.push(
                Diagnostic::warning("P220", sp, format!("`{qual}` cannot be compiled natively; importing it through CPython"))
                    .note(format!("first blocker: {first}")),
            );
            self.record(qual, "python (CPython fallback)", Some(path), Some(first));
            return None;
        }
        self.diags.extend(diags);
        self.record(qual, "python (native)", Some(path), None);
        uid
    }

    /// The units of `a`, `a.b`, `a.b.c` for an import of `a.b.c` (relative
    /// with `level` dots). `None` when the module isn't a `.py` file next to
    /// the program (a CPython import) or can't be compiled.
    fn resolve_chain(&mut self, parts: &[String], level: usize, sp: Span) -> Option<Vec<usize>> {
        let mut chain = Vec::new();
        let (mut dir, mut prefix) = if level == 0 {
            let first = parts.first()?;
            let base = self.import_bases().into_iter().find(|b| b.join(first).join("__init__.py").exists() || b.join(format!("{first}.py")).exists())?;
            (base, None)
        } else {
            let Some((mut dir, mut name)) = self.own_package() else {
                self.unsupported(sp, "relative import outside a package", "CPython raises ImportError: attempted relative import with no known parent package");
                return None;
            };
            for _ in 1..level {
                dir = dir.parent()?.to_path_buf();
                name = name.rsplit_once('.')?.0.to_string();
            }
            let init = dir.join("__init__.py");
            if init.exists() {
                chain.push(self.module_unit(&init, &name, sp)?);
            }
            (dir, Some(name))
        };
        for (i, part) in parts.iter().enumerate() {
            let qual = match &prefix {
                Some(p) => format!("{p}.{part}"),
                None => part.clone(),
            };
            let init = dir.join(part).join("__init__.py");
            let file = dir.join(format!("{part}.py"));
            let (path, is_pkg) = if init.exists() {
                (init, true)
            } else if file.exists() {
                (file, false)
            } else {
                return None;
            };
            if !is_pkg && i + 1 < parts.len() {
                return None;
            }
            let u = self.module_unit(&path, &qual, sp)?;
            if let Some(&parent) = chain.last() {
                self.ld.unit_subs[parent].insert(part.clone(), u);
            }
            chain.push(u);
            dir = dir.join(part);
            prefix = Some(qual);
        }
        Some(chain)
    }

    /// `pkg.attr` as a submodule of the package unit `u` (loaded if needed).
    fn submodule(&mut self, u: usize, attr: &str, sp: Span) -> Option<usize> {
        if let Some(&s) = self.ld.unit_subs[u].get(attr) {
            return Some(s);
        }
        let unit = &self.ld.units[u];
        if !unit.path.ends_with("__init__.py") {
            return None;
        }
        let dir = Path::new(&unit.path).parent()?.to_path_buf();
        let qual = format!("{}.{attr}", unit.name);
        let init = dir.join(attr).join("__init__.py");
        let file = dir.join(format!("{attr}.py"));
        let path = if init.exists() { init } else if file.exists() { file } else { return None };
        let s = self.module_unit(&path, &qual, sp)?;
        self.ld.unit_subs[u].insert(attr.to_string(), s);
        Some(s)
    }

    fn extern_missing(&mut self, e: usize, attr: &str, sp: Span) {
        let m = &self.ld.externs[e];
        let why = m.skipped.iter().find(|(n, _)| n == attr).map(|(_, w)| w.clone());
        let lang = if m.lang == Lang::Rust { "pub fn" } else { "function" };
        let mut d = Diagnostic::error("P150", sp, format!("`{}` has no callable {lang} `{attr}`", m.name));
        if let Some(w) = why {
            d = d.note(format!("`{attr}` exists but its signature cannot cross the boundary: {w}"));
        }
        self.diags.push(d);
    }

    fn lower_function(&mut self, id: FuncId, st: &Value) -> Function {
        let name = match &self.cur_class {
            Some((c, _)) => format!("{}.{}", self.ld.classes[*c as usize].name.rsplit('.').next().unwrap_or(""), sstr(st, "name")),
            None => sstr(st, "name").to_string(),
        };
        let sp = self.span(st);
        let a = &st["args"];
        let (_, sig) = signature(a);
        let mut params: Vec<String> = sig.names.clone();
        params.extend(sig.vararg.clone());
        params.extend(sig.kwarg.clone());
        let mut decl = HashSet::new();
        global_decls(arr(st, "body"), &mut decl);
        let mut nonlocals = HashSet::new();
        nonlocal_decls(arr(st, "body"), &mut nonlocals);
        let mut locals = params.clone();
        collect_bindings(arr(st, "body"), &mut locals, false);
        locals.retain(|n| !decl.contains(n) && !nonlocals.contains(n));
        self.fn_globals = decl;
        self.cur_fid = Some(id);
        for n in &nonlocals {
            match self.find_enclosing(n) {
                Some(k) => self.record_capture(n, k),
                None => self.err("P001", sp, format!("SyntaxError: no binding for nonlocal '{n}' found")),
            }
        }
        self.fn_nonlocals = nonlocals;
        self.locals = locals.iter().cloned().collect();
        self.in_module = false;
        let (param_ann, ret_ann) = self.ld.func_anns.get(&id).cloned().unwrap_or_default();
        self.ret_ann = ret_ann;
        let saved_bindings = self.bindings.clone();
        let saved_native = std::mem::take(&mut self.native_imported);
        let body = self.lower_block(arr(st, "body"));
        locals.retain(|n| !self.native_imported.contains(n));
        self.bindings = saved_bindings;
        self.native_imported = saved_native;
        self.ret_ann = None;
        self.fn_globals.clear();
        locals.extend(std::mem::take(&mut self.extra_locals));
        let sig = self.ld.func_sigs.get(&id).cloned().unwrap_or_default();
        let class = self.cur_class.as_ref().map(|c| c.0);
        let is_async = st["is_async"].as_bool().unwrap_or(false);
        let is_generator = arr(st, "body").iter().any(contains_yield);
        Function { id, name: self.qual(&name), params, locals, body, span: sp, is_module: false, unit: self.unit, param_ann, ret_ann, class, sig, parent: None, captures: vec![], cells: vec![], is_async, is_generator }
    }

    fn lower_block(&mut self, body: &[Value]) -> Vec<Stmt> {
        body.iter().filter_map(|s| self.lower_stmt(s)).collect()
    }

    fn name_target(&self, t: &Value) -> Option<String> {
        (ty(t) == "Name").then(|| sstr(t, "id").to_string())
    }

    fn lower_stmt(&mut self, st: &Value) -> Option<Stmt> {
        let sp = self.span(st);
        let kind = match ty(st) {
            "Expr" => StmtKind::Expr(self.lower_expr(&st["value"])?),
            "Assign" => {
                let targets = arr(st, "targets");
                let value = self.lower_expr(&st["value"])?;
                if targets.len() == 1 && matches!(ty(&targets[0]), "Tuple" | "List") {
                    let stmts = self.lower_target(&targets[0], value, sp)?;
                    return Some(self.block_stmt(stmts, sp));
                }
                if targets.len() == 1 && ty(&targets[0]) != "Name" {
                    return self.lower_store(&targets[0], value, sp);
                }
                let mut names = Vec::new();
                for t in targets {
                    match self.name_target(t) {
                        Some(n) => names.push(n),
                        None => {
                            // `a = b, c = value`: bind a temp, then each target from it.
                            let tmp = self.fresh_local();
                            let mut stmts = vec![Stmt { kind: StmtKind::Assign { target: tmp.clone(), value }, span: sp }];
                            for t in targets {
                                let read = Expr { kind: ExprKind::Local(tmp.clone()), span: sp };
                                stmts.extend(self.lower_target(t, read, sp)?);
                            }
                            return Some(self.block_stmt(stmts, sp));
                        }
                    }
                }
                if names.len() == 1 {
                    self.bind_target(names.remove(0), value)
                } else {
                    return Some(self.multi_assign(names, value, sp));
                }
            }
            "AnnAssign" => {
                if st["value"].is_null() {
                    return None; // bare annotation: no runtime effect
                }
                let value = self.lower_expr(&st["value"])?;
                // `parts: list[str] = []`: a container annotation gives the
                // (still empty) value its element types.
                let value = match self.container_ann(&st["annotation"]) {
                    Some(ty) if parse_ann(&st["annotation"]).is_none() => {
                        let span = value.span;
                        Expr { kind: ExprKind::Annotated { ty, value: Box::new(value) }, span }
                    }
                    _ => annotated(parse_ann(&st["annotation"]), value),
                };
                match self.name_target(&st["target"]) {
                    Some(target) => self.bind_target(target, value),
                    None => return self.lower_store(&st["target"], value, sp),
                }
            }
            "AugAssign" => {
                let op = self.binop(&st["op"], sp)?;
                let value = self.lower_expr(&st["value"])?;
                let t = &st["target"];
                match self.name_target(t) {
                    Some(target) if !self.locals.contains(&target) => {
                        let cur = self.lower_name(&target, sp)?;
                        let v = Expr { kind: ExprKind::BinOp { op, left: Box::new(cur), right: Box::new(value) }, span: sp };
                        self.bind_target(target, v)
                    }
                    Some(target) => StmtKind::AugAssign { target, op, value },
                    None => {
                        // obj[i] += v / obj.a += v: only for side-effect-free receivers,
                        // since the receiver is evaluated twice here.
                        let simple = |v: &Value| matches!(ty(v), "Name" | "Constant");
                        let ok = match ty(t) {
                            "Subscript" => simple(&t["value"]) && simple(&t["slice"]),
                            "Attribute" => simple(&t["value"]),
                            _ => false,
                        };
                        if !ok {
                            // `a[f()][k] += v`: evaluate the receiver (and index) once,
                            // into temporaries, as CPython does.
                            if !matches!(ty(t), "Subscript" | "Attribute") {
                                self.unsupported(sp, "augmented assignment to this target", "use a temporary variable");
                                return None;
                            }
                            let loc = &t["_loc"];
                            let mut stmts = Vec::new();
                            let mut t2 = t.clone();
                            let obj_tmp = self.fresh_local();
                            let obj = self.lower_expr(&t["value"])?;
                            stmts.push(Stmt { kind: StmtKind::Assign { target: obj_tmp.clone(), value: obj }, span: sp });
                            t2["value"] = serde_json::json!({"_t": "Name", "id": obj_tmp, "ctx": {"_t": "Load"}, "_loc": loc});
                            if ty(t) == "Subscript" && !simple(&t["slice"]) && ty(&t["slice"]) != "Slice" {
                                let idx_tmp = self.fresh_local();
                                let idx = self.lower_expr(&t["slice"])?;
                                stmts.push(Stmt { kind: StmtKind::Assign { target: idx_tmp.clone(), value: idx }, span: sp });
                                t2["slice"] = serde_json::json!({"_t": "Name", "id": idx_tmp, "ctx": {"_t": "Load"}, "_loc": loc});
                            }
                            let cur = self.lower_expr(&t2)?;
                            let v = Expr { kind: ExprKind::BinOp { op, left: Box::new(cur), right: Box::new(value) }, span: sp };
                            stmts.push(self.lower_store(&t2, v, sp)?);
                            return Some(self.block_stmt(stmts, sp));
                        }
                        let cur = self.lower_expr(t)?;
                        let v = Expr { kind: ExprKind::BinOp { op, left: Box::new(cur), right: Box::new(value) }, span: sp };
                        return self.lower_store(t, v, sp);
                    }
                }
            }
            "Return" => {
                if self.in_module {
                    self.err("P001", sp, "SyntaxError: 'return' outside function");
                    return None;
                }
                let ann = self.ret_ann;
                StmtKind::Return(if st["value"].is_null() { None } else { Some(annotated(ann, self.lower_expr(&st["value"])?)) })
            }
            "If" => StmtKind::If {
                test: self.lower_expr(&st["test"])?,
                body: self.lower_block(arr(st, "body")),
                orelse: self.lower_block(arr(st, "orelse")),
            },
            "While" => {
                if !arr(st, "orelse").is_empty() {
                    self.unsupported(sp, "`while ... else`", "move the else-body after the loop with an explicit flag");
                    return None;
                }
                let test = self.lower_expr(&st["test"])?;
                StmtKind::While { test, body: self.lower_block(arr(st, "body")) }
            }
            "For" => return self.lower_for(st),
            "Break" => StmtKind::Break,
            "Continue" => StmtKind::Continue,
            "Pass" => StmtKind::Pass,
            "FunctionDef" => {
                // A nested function: a closure value bound to its name.
                let name = sstr(st, "name").to_string();
                let fid = self.ld.alloc_func();
                let (params, sig) = signature(&st["args"]);
                let mut anns: Vec<Option<Ty>> = params.iter().map(|p| parse_ann(&p["annotation"])).collect();
                anns.extend(sig.vararg.iter().chain(&sig.kwarg).map(|_| None));
                self.ld.func_anns.insert(fid, (anns, parse_ann(&st["returns"])));
                self.ld.func_sigs.insert(fid, call_sig(&sig, self.unit));
                let mut value = self.lower_nested(fid, st)?;
                for d in arr(st, "decorator_list").iter().rev() {
                    let dec = self.lower_expr(d)?;
                    value = Expr { kind: ExprKind::CallObj { func: Box::new(dec), args: vec![value], kwargs: vec![] }, span: sp };
                }
                self.bind_target(name, value)
            }
            "Import" | "ImportFrom" if !self.import_actions.contains_key(&sp) && !self.var_imports.contains_key(&sp) => {
                return self.lower_nested_import(st);
            }
            "Import" | "ImportFrom" if self.missing_imports.contains_key(&sp) => {
                let name = self.missing_imports.remove(&sp).unwrap();
                self.import_actions.remove(&sp);
                let msg = Expr { kind: ExprKind::Str(format!("No module named '{name}'")), span: sp };
                let exc = Expr { kind: ExprKind::NewExc { kind: "ModuleNotFoundError".into(), args: vec![msg] }, span: sp };
                StmtKind::Raise(Some(exc))
            }
            "Import" | "ImportFrom" => {
                let actions = self.import_actions.remove(&sp).unwrap_or_default();
                let mut stmts: Vec<Stmt> = actions.into_iter().map(|a| Stmt { kind: StmtKind::Import(a), span: sp }).collect();
                for (bind, u, name) in self.var_imports.remove(&sp).unwrap_or_default() {
                    let value = Expr { kind: ExprKind::Global { unit: u, name }, span: sp };
                    stmts.push(Stmt { kind: self.bind_target(bind, value), span: sp });
                }
                return match stmts.len() {
                    0 => None,
                    1 => stmts.into_iter().next(),
                    _ => Some(Stmt { kind: StmtKind::If { test: Expr { kind: ExprKind::Bool(true), span: sp }, body: stmts, orelse: vec![] }, span: sp }),
                };
            }
            "Try" => {
                let body = self.lower_block(arr(st, "body"));
                let mut handlers = Vec::new();
                for h in arr(st, "handlers") {
                    let hsp = self.span(h);
                    let types = match ty(&h["type"]) {
                        "" => vec![],
                        "Tuple" => {
                            let mut v = Vec::new();
                            for t in arr(&h["type"], "elts") {
                                v.push(self.exc_type(t)?);
                            }
                            v
                        }
                        _ => vec![self.exc_type(&h["type"])?],
                    };
                    // CPython deletes the `as` name when the handler ends, so each
                    // handler's binding is its own local (its type can differ).
                    let renamed = h["name"].as_str().filter(|n| self.locals.contains(*n) && !self.globals.contains(*n)).map(|n| {
                        self.tmp += 1;
                        let r = format!("__h{}_{n}", self.tmp);
                        self.locals.insert(r.clone());
                        self.extra_locals.push(r.clone());
                        (n.to_string(), r)
                    });
                    let (name, prefix) = match (&renamed, h["name"].as_str()) {
                        (Some((_, r)), _) => (Some(r.clone()), None),
                        (None, Some(n)) => {
                            let (n, p) = self.local_target(n.to_string(), hsp);
                            (Some(n), p)
                        }
                        (None, None) => (None, None),
                    };
                    if let Some((orig, r)) = &renamed {
                        self.comp_scopes.push(HashMap::from([(orig.clone(), r.clone())]));
                        self.comp_owner.push(self.cur_fid);
                    }
                    let mut hbody = self.lower_block(arr(h, "body"));
                    if renamed.is_some() {
                        self.comp_scopes.pop();
                        self.comp_owner.pop();
                    }
                    if let Some(p) = prefix {
                        hbody.insert(0, p);
                    }
                    handlers.push(Handler { types, name, body: hbody, span: hsp });
                }
                StmtKind::Try { body, handlers, orelse: self.lower_block(arr(st, "orelse")), finalbody: self.lower_block(arr(st, "finalbody")) }
            }
            "TryStar" => {
                self.unsupported(sp, "`except*`", "exception groups are not compiled; use plain `except`");
                return None;
            }
            "Raise" => {
                if st["exc"].is_null() {
                    StmtKind::Raise(None)
                } else {
                    let e = &st["exc"];
                    // `raise ValueError` raises an instance of the class.
                    if ty(e) == "Name" && self.builtin_exc(sstr(e, "id")) {
                        StmtKind::Raise(Some(Expr { kind: ExprKind::NewExc { kind: sstr(e, "id").to_string(), args: vec![] }, span: sp }))
                    } else if let (true, Some(Binding::Class(c))) = (ty(e) == "Name" && !self.locals.contains(sstr(e, "id")), self.bindings.get(sstr(e, "id")).cloned()) {
                        StmtKind::Raise(Some(Expr { kind: ExprKind::New { class: c, args: vec![], kwargs: vec![] }, span: sp }))
                    } else {
                        StmtKind::Raise(Some(self.lower_expr(e)?))
                    }
                }
            }
            "Assert" => {
                let test = self.lower_expr(&st["test"])?;
                let args = if st["msg"].is_null() { vec![] } else { vec![self.lower_expr(&st["msg"])?] };
                let raise = Stmt { kind: StmtKind::Raise(Some(Expr { kind: ExprKind::NewExc { kind: "AssertionError".into(), args }, span: sp })), span: sp };
                let not = Expr { kind: ExprKind::Unary { op: UnaryOp::Not, operand: Box::new(test) }, span: sp };
                StmtKind::If { test: not, body: vec![raise], orelse: vec![] }
            }
            "With" => {
                let items = arr(st, "items");
                let mut body = self.lower_block(arr(st, "body"));
                for item in items.iter().rev() {
                    let ctx = self.lower_expr(&item["context_expr"])?;
                    let var = match ty(&item["optional_vars"]) {
                        "" => None,
                        "Name" => {
                            let (v, p) = self.local_target(sstr(&item["optional_vars"], "id").to_string(), sp);
                            if let Some(p) = p {
                                body.insert(0, p);
                            }
                            Some(v)
                        }
                        _ => {
                            let tmp = self.fresh_local();
                            let read = Expr { kind: ExprKind::Local(tmp.clone()), span: sp };
                            let stmts = self.lower_target(&item["optional_vars"], read, sp)?;
                            body.insert(0, self.block_stmt(stmts, sp));
                            Some(tmp)
                        }
                    };
                    let is_async = st["is_async"].as_bool().unwrap_or(false);
                    body = vec![Stmt { kind: StmtKind::With { ctx, var, body, is_async }, span: sp }];
                }
                return body.into_iter().next();
            }
            "Global" => return None, // handled by the pre-scan
            "Delete" => {
                let mut stmts = Vec::new();
                for t in arr(st, "targets") {
                    let tsp = self.span(t);
                    match ty(t) {
                        "Subscript" => {
                            let obj = self.lower_expr(&t["value"])?;
                            let index = self.lower_index(&t["slice"])?;
                            stmts.push(Stmt { kind: StmtKind::Del(Expr { kind: ExprKind::Subscript { obj: Box::new(obj), index: Box::new(index) }, span: tsp }), span: tsp });
                        }
                        "Attribute" => {
                            let obj = self.lower_expr(&t["value"])?;
                            let name = sstr(t, "attr").to_string();
                            stmts.push(Stmt { kind: StmtKind::Del(Expr { kind: ExprKind::GetAttr { obj: Box::new(obj), name }, span: tsp }), span: tsp });
                        }
                        // `del x`: the variable is unbound again (a later read
                        // raises UnboundLocalError / NameError, as in CPython).
                        "Name" => {
                            let target = self.lower_name(sstr(t, "id"), tsp)?;
                            match target.kind {
                                ExprKind::Local(_) | ExprKind::Global { .. } => stmts.push(Stmt { kind: StmtKind::Del(target), span: tsp }),
                                _ => {
                                    self.unsupported(tsp, "`del` of this name", "rebind it instead (e.g. to None)");
                                    return None;
                                }
                            }
                        }
                        _ => {
                            self.unsupported(tsp, "`del` of this target", "delete names, items or attributes");
                            return None;
                        }
                    }
                }
                return match stmts.len() {
                    1 => stmts.pop(),
                    _ => Some(Stmt { kind: StmtKind::If { test: Expr { kind: ExprKind::Bool(true), span: sp }, body: stmts, orelse: vec![] }, span: sp }),
                };
            }
            "Nonlocal" if !self.in_module => return None, // handled by the pre-scan
            "Nonlocal" => {
                self.err("P142", sp, "`global`/`nonlocal` rebinding is never lowered into a native assumption");
                return None;
            }
            other => {
                self.unsupported(sp, &format!("statement `{other}`"), "see docs/subset.md for the supported subset");
                return None;
            }
        };
        Some(Stmt { kind, span: sp })
    }

    /// `obj[i] = v` / `obj.attr = v` (through CPython).
    fn lower_store(&mut self, t: &Value, value: Expr, sp: Span) -> Option<Stmt> {
        let kind = match ty(t) {
            "Subscript" => StmtKind::SetItem { obj: self.lower_expr(&t["value"])?, index: self.lower_index(&t["slice"])?, value },
            "Attribute" if ty(&t["value"]) == "Name" && !self.locals.contains(sstr(&t["value"], "id")) && matches!(self.bindings.get(sstr(&t["value"], "id")), Some(Binding::Class(_))) => {
                let Some(Binding::Class(c)) = self.bindings.get(sstr(&t["value"], "id")).cloned() else { unreachable!() };
                let attr = sstr(t, "attr");
                match self.ld.classes.iter().position(|_| true).and(Some(())).and_then(|_| self.class_var_owner(c, attr)) {
                    Some((u, qual)) => StmtKind::AssignGlobal { unit: u, name: qual, value },
                    None => {
                        self.unsupported(sp, &format!("adding class attribute `{attr}` after the class statement"), "declare it in the class body");
                        return None;
                    }
                }
            }
            "Attribute" => match self.module_binding(&t["value"]) {
                Some(Binding::Unit(u)) if self.ld.unit_globals[u].contains(sstr(t, "attr")) => {
                    StmtKind::AssignGlobal { unit: u, name: sstr(t, "attr").to_string(), value }
                }
                _ => StmtKind::SetAttr { obj: self.lower_expr(&t["value"])?, name: sstr(t, "attr").to_string(), value },
            },
            other => {
                let tsp = self.span(t);
                self.unsupported(tsp, &format!("assignment to `{other}`"), "tuple unpacking is not compiled yet");
                return None;
            }
        };
        Some(Stmt { kind, span: sp })
    }

    fn multi_assign(&mut self, names: Vec<String>, value: Expr, sp: Span) -> Stmt {
        let first = names[0].clone();
        let mut stmts = vec![Stmt { kind: self.bind_target(first.clone(), value), span: sp }];
        for n in &names[1..] {
            let read = match self.lower_name(&first, sp) {
                Some(r) => r,
                None => break,
            };
            stmts.push(Stmt { kind: self.bind_target(n.clone(), read), span: sp });
        }
        // Encode as an always-true `if` block to keep a single statement slot.
        Stmt { kind: StmtKind::If { test: Expr { kind: ExprKind::Bool(true), span: sp }, body: stmts, orelse: vec![] }, span: sp }
    }

    fn lower_for(&mut self, st: &Value) -> Option<Stmt> {
        let sp = self.span(st);
        if !arr(st, "orelse").is_empty() {
            self.unsupported(sp, "`for ... else`", "move the else-body after the loop with an explicit flag");
            return None;
        }
        let (var, prefix) = match self.name_target(&st["target"]) {
            Some(var) => self.local_target(var, sp),
            None => {
                let tmp = self.fresh_local();
                let read = Expr { kind: ExprKind::Local(tmp.clone()), span: sp };
                let stmts = self.lower_target(&st["target"], read, sp)?;
                (tmp, Some(self.block_stmt(stmts, sp)))
            }
        };
        let with_prefix = |mut body: Vec<Stmt>| {
            if let Some(p) = prefix.clone() {
                body.insert(0, p);
            }
            body
        };
        let it_owned = self.enum_members_json(&st["iter"]);
        let it = &it_owned;
        let is_range = ty(it) == "Call"
            && ty(&it["func"]) == "Name"
            && it["func"]["id"] == "range"
            && !self.locals.contains("range")
            && !self.bindings.contains_key("range")
            && arr(it, "keywords").is_empty()
            && (1..=3).contains(&arr(it, "args").len());
        let is_async = st["is_async"].as_bool().unwrap_or(false);
        if !is_range || is_async {
            let iter = self.lower_expr(it)?;
            let body = with_prefix(self.lower_block(arr(st, "body")));
            return Some(Stmt { kind: StmtKind::ForIter { var, iter, body, is_async }, span: sp });
        }
        let mut lowered = Vec::new();
        for a in arr(it, "args") {
            lowered.push(self.lower_expr(a)?);
        }
        let (start, stop, step) = match lowered.len() {
            1 => (None, lowered.remove(0), None),
            2 => {
                let stop = lowered.pop().unwrap();
                (Some(lowered.remove(0)), stop, None)
            }
            _ => {
                let step = lowered.pop().unwrap();
                let stop = lowered.pop().unwrap();
                (Some(lowered.remove(0)), stop, Some(step))
            }
        };
        let body = with_prefix(self.lower_block(arr(st, "body")));
        Some(Stmt { kind: StmtKind::ForRange { var, start, stop, step, body }, span: sp })
    }

    fn binop(&mut self, op: &Value, sp: Span) -> Option<BinOp> {
        Some(match ty(op) {
            "Add" => BinOp::Add,
            "Sub" => BinOp::Sub,
            "Mult" => BinOp::Mul,
            "Div" => BinOp::Div,
            "FloorDiv" => BinOp::FloorDiv,
            "MatMult" => BinOp::MatMul,
            "Mod" => BinOp::Mod,
            "Pow" => BinOp::Pow,
            "BitAnd" => BinOp::BitAnd,
            "BitOr" => BinOp::BitOr,
            "BitXor" => BinOp::BitXor,
            "LShift" => BinOp::LShift,
            "RShift" => BinOp::RShift,
            other => {
                self.unsupported(sp, &format!("operator `{other}`"), "matrix multiplication is not wired up yet");
                return None;
            }
        })
    }

    fn py_builtin(&self, name: &str, sp: Span) -> Expr {
        Expr { kind: ExprKind::PyBuiltin(name.into()), span: sp }
    }

    fn call_obj(&self, func: Expr, args: Vec<Expr>, sp: Span) -> Expr {
        Expr { kind: ExprKind::CallObj { func: Box::new(func), args, kwargs: vec![] }, span: sp }
    }

    fn lower_index(&mut self, idx: &Value) -> Option<Expr> {
        // `a[:, 4]`, `a[1:3, ::2]` (numpy): a tuple whose items may be slices.
        if ty(idx) == "Tuple" && arr(idx, "elts").iter().any(|x| ty(x) == "Slice") {
            let sp = self.span(idx);
            let mut items = Vec::new();
            for x in arr(idx, "elts") {
                items.push(self.lower_index(x)?);
            }
            return Some(Expr { kind: ExprKind::Tuple(items), span: sp });
        }
        if ty(idx) != "Slice" {
            return self.lower_expr(idx);
        }
        let sp = self.span(idx);
        let part = |me: &mut Self, k: &str| -> Option<Option<Box<Expr>>> {
            Some(if idx[k].is_null() { None } else { Some(Box::new(me.lower_expr(&idx[k])?)) })
        };
        let lo = part(self, "lower")?;
        let hi = part(self, "upper")?;
        let step = part(self, "step")?;
        Some(Expr { kind: ExprKind::Slice { lo, hi, step }, span: sp })
    }

    fn lower_exprs(&mut self, vs: &[Value]) -> Option<Vec<Expr>> {
        let mut out = Vec::new();
        for v in vs {
            if ty(v) == "Starred" {
                let sp = self.span(v);
                self.unsupported(sp, "`*` unpacking", "build the collection explicitly");
                return None;
            }
            out.push(self.lower_expr(v)?);
        }
        Some(out)
    }

    fn lower_expr(&mut self, e: &Value) -> Option<Expr> {
        let sp = self.span(e);
        let kind = match ty(e) {
            "Constant" => {
                let c = &e["value"];
                match c["kind"].as_str().unwrap_or("") {
                    "none" => ExprKind::NoneLit,
                    "bool" => ExprKind::Bool(c["value"].as_bool().unwrap_or(false)),
                    "int" => {
                        let s = c["value"].as_str().unwrap_or("0");
                        match s.parse::<i64>() {
                            Ok(v) => ExprKind::Int(v),
                            Err(_) => {
                                self.diags.push(
                                    Diagnostic::error("P110", sp, format!("integer literal {s} is outside the i64 subset"))
                                        .note("arbitrary-precision ints are milestone M4"),
                                );
                                return None;
                            }
                        }
                    }
                    "float" => ExprKind::Float(c["value"].as_str().unwrap_or("0.0").parse().unwrap_or(f64::NAN)),
                    "str" => ExprKind::Str(sstr(c, "value").to_string()),
                    // `b"..."` is `bytes([...])`, a CPython object.
                    "bytes" => {
                        let items = c["value"].as_array().map(|v| v.iter().map(|b| Expr { kind: ExprKind::Int(b.as_i64().unwrap_or(0)), span: sp }).collect()).unwrap_or_default();
                        ExprKind::CallObj {
                            func: Box::new(Expr { kind: ExprKind::PyBuiltin("bytes".into()), span: sp }),
                            args: vec![Expr { kind: ExprKind::List(items), span: sp }],
                            kwargs: vec![],
                        }
                    }
                    _ => {
                        self.unsupported(sp, &format!("`{}` literal", sstr(c, "type")), "bytes/complex/Ellipsis are not supported yet");
                        return None;
                    }
                }
            }
            "Name" => return self.lower_name(sstr(e, "id"), sp),
            "BinOp" => {
                let op = self.binop(&e["op"], sp)?;
                let left = self.lower_expr(&e["left"])?;
                let right = self.lower_expr(&e["right"])?;
                ExprKind::BinOp { op, left: Box::new(left), right: Box::new(right) }
            }
            "Await" => ExprKind::Await(Box::new(self.lower_expr(&e["value"])?)),
            "YieldFrom" => ExprKind::YieldFrom(Box::new(self.lower_expr(&e["value"])?)),
            "Yield" => ExprKind::Yield(match e["value"].is_null() {
                true => None,
                false => Some(Box::new(self.lower_expr(&e["value"])?)),
            }),
            "UnaryOp" => {
                let op = match ty(&e["op"]) {
                    "USub" => UnaryOp::Neg,
                    "UAdd" => UnaryOp::Pos,
                    "Not" => UnaryOp::Not,
                    _ => UnaryOp::Invert,
                };
                // Fold `-<int literal>` so that i64::MIN is representable.
                if op == UnaryOp::Neg && ty(&e["operand"]) == "Constant" && e["operand"]["value"]["kind"] == "int" {
                    let s = format!("-{}", e["operand"]["value"]["value"].as_str().unwrap_or("0"));
                    if let Ok(v) = s.parse::<i64>() {
                        return Some(Expr { kind: ExprKind::Int(v), span: sp });
                    }
                }
                ExprKind::Unary { op, operand: Box::new(self.lower_expr(&e["operand"])?) }
            }
            "Compare" => {
                let first = self.lower_expr(&e["left"])?;
                let mut rest = Vec::new();
                for (op, c) in arr(e, "ops").iter().zip(arr(e, "comparators")) {
                    let op = match ty(op) {
                        "Eq" => CmpOp::Eq,
                        "NotEq" => CmpOp::NotEq,
                        "Lt" => CmpOp::Lt,
                        "LtE" => CmpOp::LtE,
                        "Gt" => CmpOp::Gt,
                        "GtE" => CmpOp::GtE,
                        "In" => CmpOp::In,
                        "NotIn" => CmpOp::NotIn,
                        "Is" => CmpOp::Is,
                        _ => CmpOp::IsNot,
                    };
                    let c = if matches!(op, CmpOp::In | CmpOp::NotIn) { self.enum_members_json(c) } else { c.clone() };
                    rest.push((op, self.lower_expr(&c)?));
                }
                ExprKind::Compare { first: Box::new(first), rest }
            }
            "BoolOp" => {
                let op = if ty(&e["op"]) == "And" { BoolOpKind::And } else { BoolOpKind::Or };
                ExprKind::BoolOp { op, values: self.lower_exprs(arr(e, "values"))? }
            }
            "NamedExpr" => {
                let name = sstr(&e["target"], "id").to_string();
                let value = self.lower_expr(&e["value"])?;
                if !self.locals.contains(&name) && !self.fn_globals.contains(&name) && !self.globals.contains(&name) && !self.fn_nonlocals.contains(&name) {
                    // Inside a comprehension, `:=` binds the enclosing scope.
                    self.unsupported(sp, "`:=` inside a comprehension or lambda", "assign the value in a statement before it");
                    return None;
                }
                match self.bind_target(name, value) {
                    StmtKind::Assign { target, value } => ExprKind::Walrus { target, global: None, value: Box::new(value) },
                    StmtKind::AssignGlobal { unit, name, value } => ExprKind::Walrus { target: name, global: Some(unit), value: Box::new(value) },
                    _ => {
                        self.unsupported(sp, "`:=` on this variable", "assign it in a statement");
                        return None;
                    }
                }
            }
            "IfExp" => ExprKind::IfExp {
                test: Box::new(self.lower_expr(&e["test"])?),
                body: Box::new(self.lower_expr(&e["body"])?),
                orelse: Box::new(self.lower_expr(&e["orelse"])?),
            },
            "Call" => return self.lower_call(e),
            "JoinedStr" => return self.lower_fstring(e),
            "Attribute" => {
                let attr = sstr(e, "attr");
                if ty(&e["value"]) == "Name" && !self.locals.contains(sstr(&e["value"], "id")) {
                    if let Some(Binding::Class(c)) = self.bindings.get(sstr(&e["value"], "id")).cloned() {
                        return self.class_attr(c, attr, sp);
                    }
                }
                if let Some(Binding::Unit(u)) = self.module_binding(&e["value"]) {
                    if let Some(&c) = self.ld.unit_classes[u].get(attr) {
                        return Some(Expr { kind: ExprKind::ClassRef(c), span: sp });
                    }
                    if self.ld.unit_globals[u].contains(attr) {
                        return Some(Expr { kind: ExprKind::Global { unit: u, name: attr.to_string() }, span: sp });
                    }
                }
                if let Some(Binding::Unit(u)) = self.module_binding(&e["value"]) {
                    if let Some(&f) = self.ld.unit_funcs[u].get(attr) {
                        return Some(Expr { kind: ExprKind::MakeClosure { func: f, captures: vec![], defaults: vec![] }, span: sp });
                    }
                    if self.ld.unit_bindings[u].contains_key(attr) && !matches!(self.ld.unit_bindings[u][attr], Binding::Unit(_) | Binding::Extern(_)) {
                        let n = self.reexport_name(u, attr, &e["_loc"]);
                        return self.lower_expr(&n);
                    }
                }
                if let Some(b) = self.module_binding(&e["value"]) {
                    let what = match b {
                        Binding::Unit(u) if !self.ld.unit_funcs[u].contains_key(attr) => "module-level variable of a natively compiled module",
                        _ => "function used as a first-class value",
                    };
                    self.unsupported(sp, &format!("`{attr}` ({what})"), "call the function directly; module globals are not compiled yet");
                    return None;
                }
                ExprKind::GetAttr { obj: Box::new(self.lower_expr(&e["value"])?), name: attr.to_string() }
            }
            "List" => ExprKind::List(self.lower_exprs(arr(e, "elts"))?),
            "Tuple" => ExprKind::Tuple(self.lower_exprs(arr(e, "elts"))?),
            "Set" => ExprKind::Set(self.lower_exprs(arr(e, "elts"))?),
            "Dict" => {
                let mut pairs = Vec::new();
                for (k, v) in arr(e, "keys").iter().zip(arr(e, "values")) {
                    if k.is_null() {
                        self.unsupported(sp, "`**` in a dict literal", "merge dicts explicitly");
                        return None;
                    }
                    pairs.push((self.lower_expr(k)?, self.lower_expr(v)?));
                }
                ExprKind::Dict(pairs)
            }
            // `Color["RED"]`: the member with that name.
            "Subscript" if self.enum_class_of(&e["value"]).is_some() => {
                let call = serde_json::json!({"_t": "Call", "func": {"_t": "Attribute", "value": e["value"], "attr": "_pyrock_by_name_", "ctx": {"_t": "Load"}, "_loc": e["_loc"]}, "args": [e["slice"]], "keywords": [], "_loc": e["_loc"]});
                return self.lower_call(&call);
            }
            "Subscript" => ExprKind::Subscript { obj: Box::new(self.lower_expr(&e["value"])?), index: Box::new(self.lower_index(&e["slice"])?) },
            "ListComp" | "SetComp" | "GeneratorExp" | "DictComp" => return self.lower_comp(e),
            "Lambda" => {
                let fid = self.ld.alloc_func();
                let (params, sig) = signature(&e["args"]);
                let anns: Vec<Option<Ty>> = params.iter().map(|_| None).chain(sig.vararg.iter().chain(&sig.kwarg).map(|_| None)).collect();
                self.ld.func_anns.insert(fid, (anns, None));
                self.ld.func_sigs.insert(fid, call_sig(&sig, self.unit));
                let def = serde_json::json!({
                    "_t": "FunctionDef", "name": "<lambda>", "args": e["args"], "decorator_list": [], "returns": null, "_loc": e["_loc"],
                    "body": [{"_t": "Return", "value": e["body"], "_loc": e["_loc"]}],
                });
                return self.lower_nested(fid, &def);
            }
            other => {
                self.unsupported(sp, &format!("expression `{other}`"), "see docs/subset.md for the supported subset");
                return None;
            }
        };
        Some(Expr { kind, span: sp })
    }

    /// The unit and global-cell name of class attribute `attr` of `c` (or a base).
    fn class_var_owner(&self, c: ClassId, attr: &str) -> Option<(usize, String)> {
        for k in c3_mro(&self.ld.classes, c) {
            let cd = &self.ld.classes[k as usize];
            if cd.class_vars.iter().any(|v| v == attr) {
                let short = cd.name.rsplit('.').next().unwrap_or("");
                return Some((cd.unit, format!("{short}.{attr}")));
            }
        }
        None
    }

    /// `Point.attr` (a class attribute, or the class's method used as a value).
    fn class_attr(&mut self, c: ClassId, attr: &str, sp: Span) -> Option<Expr> {
        if let Some((u, qual)) = self.class_var_owner(c, attr) {
            return Some(Expr { kind: ExprKind::Global { unit: u, name: qual }, span: sp });
        }
        let cname = self.ld.classes[c as usize].name.clone();
        self.unsupported(sp, &format!("`{cname}.{attr}` as a value"), "call methods directly; class attributes must be assigned in the class body");
        None
    }

    fn method_of(&self, c: ClassId, name: &str) -> Option<Method> {
        c3_mro(&self.ld.classes, c).into_iter().find_map(|k| self.ld.classes[k as usize].methods.iter().find(|m| m.name == name).cloned())
    }

    /// `Point.make(...)` / `Point.norm(p)`.
    fn lower_class_attr_call(&mut self, c: ClassId, attr: &str, args: Vec<Expr>, kwargs: Vec<(String, Expr)>, sp: Span) -> Option<Expr> {
        let cname = self.ld.classes[c as usize].name.clone();
        let Some(m) = self.method_of(c, attr) else {
            if self.class_var_owner(c, attr).is_some() {
                let f = self.class_attr(c, attr, sp)?;
                return Some(Expr { kind: ExprKind::CallObj { func: Box::new(f), args, kwargs }, span: sp });
            }
            self.err("P301", sp, format!("type object '{cname}' has no attribute '{attr}'"));
            return None;
        };
        let mut args = args;
        if m.kind == MethodKind::Class {
            args.insert(0, Expr { kind: ExprKind::ClassRef(c), span: sp });
        }
        let (args, order) = self.bind_args(m.func, &format!("{cname}.{attr}"), args, kwargs, sp)?;
        let args = self.annotate_args(m.func, args);
        Some(Expr { kind: ExprKind::Call { callee: Callee::Func(m.func), args, order }, span: sp })
    }

    /// `super().name(...)` inside a method of a class with a native base.
    fn lower_super_call(&mut self, name: &str, args: Vec<Expr>, kwargs: Vec<(String, Expr)>, sp: Span) -> Option<Expr> {
        let Some((c, self_name)) = self.cur_class.clone() else {
            self.err("P001", sp, "RuntimeError: super(): no arguments outside a method");
            return None;
        };
        // The next class after `c` in `c`'s MRO that defines `name` (with
        // mixins: `class C(Mixin, Base)` reaches Mixin, then Base).
        let mro = c3_mro(&self.ld.classes, c);
        let next = mro[1..].iter().find_map(|k| self.ld.classes[*k as usize].methods.iter().find(|m| m.name == name).cloned());
        let base = self.ld.classes[c as usize].base.clone();
        match base {
            Some(ClassBase::Native(b)) => {
                let _ = b;
                let Some(m) = next else {
                    if name == "__init__" && self.exception_root(b).is_some() {
                        return self.exc_super_init(self_name, args, sp);
                    }
                    if name == "__init__" {
                        // object.__init__: nothing to do (arguments are still evaluated).
                        return Some(Expr { kind: ExprKind::Tuple(args), span: sp });
                    }
                    self.err("P301", sp, format!("'super' object has no attribute '{name}'"));
                    return None;
                };
                let call = |me: &mut Self, m: &Method| -> Option<Expr> {
                    let mut args = args.clone();
                    match m.kind {
                        MethodKind::Instance | MethodKind::Property | MethodKind::Setter => {
                            let Some(sn) = self_name.clone() else {
                                me.err("P301", sp, "super() in a method without `self`");
                                return None;
                            };
                            args.insert(0, Expr { kind: ExprKind::Local(sn), span: sp });
                        }
                        MethodKind::Class => args.insert(0, Expr { kind: ExprKind::ClassRef(c), span: sp }),
                        MethodKind::Static => {}
                    }
                    let (args, order) = me.bind_args(m.func, &format!("super().{name}"), args, kwargs.clone(), sp)?;
                    Some(Expr { kind: ExprKind::Call { callee: Callee::Func(m.func), args, order }, span: sp })
                };
                let default = call(self, &m)?;
                // In a subclass's MRO the class after `c` can differ (`class
                // D(B, C)`: `super()` in B reaches C for a D): the receiver's
                // class picks the call.
                let mut alts: Vec<(ClassId, Method, usize)> = Vec::new();
                if m.kind == MethodKind::Instance && self_name.is_some() {
                    for r in 0..self.ld.classes.len() as ClassId {
                        let rm = c3_mro(&self.ld.classes, r);
                        let Some(pos) = rm.iter().position(|k| *k == c).filter(|_| r != c) else { continue };
                        let mm = rm[pos + 1..].iter().find_map(|k| self.ld.classes[*k as usize].methods.iter().find(|x| x.name == name).cloned());
                        if let Some(mm) = mm.filter(|mm| mm.func != m.func) {
                            alts.push((r, mm, rm.len()));
                        }
                    }
                }
                if alts.is_empty() {
                    return Some(default);
                }
                alts.sort_by(|a, b| b.2.cmp(&a.2));
                let mut out = Vec::new();
                for (r, mm, _) in alts {
                    out.push((r, call(self, &mm)?));
                }
                Some(Expr { kind: ExprKind::SuperDispatch { recv: self_name.clone().unwrap(), alts: out, default: Box::new(default) }, span: sp })
            }
            Some(ClassBase::Exception(_)) if name == "__init__" => {
                let _ = kwargs;
                self.exc_super_init(self_name, args, sp)
            }
            _ if name == "__init__" => Some(Expr { kind: ExprKind::Tuple(args), span: sp }),
            _ => {
                self.err("P301", sp, format!("'super' object has no attribute '{name}'"));
                None
            }
        }
    }

    /// The builtin exception at the root of `c`'s bases, if any.
    fn exception_root(&self, c: ClassId) -> Option<String> {
        c3_mro(&self.ld.classes, c).into_iter().find_map(|k| match &self.ld.classes[k as usize].base {
            Some(ClassBase::Exception(e)) => Some(e.clone()),
            _ => None,
        })
    }

    /// `BaseException.__init__(self, *args)`: sets the message (`str(e)`).
    fn exc_super_init(&mut self, self_name: Option<String>, args: Vec<Expr>, sp: Span) -> Option<Expr> {
        let sn = self_name?;
        let msg = match args.len() {
            0 => Expr { kind: ExprKind::Str(String::new()), span: sp },
            1 => Expr { kind: ExprKind::Call { callee: Callee::Builtin(Builtin::Str), args: vec![args[0].clone()], order: vec![] }, span: sp },
            _ => {
                let t = Expr { kind: ExprKind::Tuple(args), span: sp };
                Expr { kind: ExprKind::CallObj { func: Box::new(self.py_builtin("repr", sp)), args: vec![t], kwargs: vec![] }, span: sp }
            }
        };
        Some(Expr {
            kind: ExprKind::CallObj {
                func: Box::new(Expr { kind: ExprKind::GetAttr { obj: Box::new(Expr { kind: ExprKind::Local(sn), span: sp }), name: "__exc_init__".into() }, span: sp }),
                args: vec![msg],
                kwargs: vec![],
            },
            span: sp,
        })
    }

    /// Pass 1: a class statement's shape (methods, fields, class attributes).
    fn declare_class(&mut self, st: &Value) {
        let name = sstr(st, "name").to_string();
        let sp = self.span(st);
        if self.class_ids.contains_key(&name) || self.func_ids.contains_key(&name) {
            self.err("P141", sp, format!("class `{name}` is redefined; global rebinding is not compiled natively"));
            return;
        }
        let cid = self.ld.classes.len() as ClassId;
        let mut dataclass = false;
        let mut dc = DataclassOpts::default();
        for d in arr(st, "decorator_list") {
            let target = if ty(d) == "Call" { &d["func"] } else { d };
            let dname = if ty(target) == "Name" { sstr(target, "id") } else { sstr(target, "attr") };
            if dname == "dataclass" {
                dataclass = true;
                for k in arr(d, "keywords") {
                    let v = &k["value"];
                    let b = (ty(v) == "Constant" && v["value"]["kind"] == "bool").then(|| v["value"]["value"].as_bool().unwrap_or(false));
                    match (sstr(k, "arg"), b) {
                        ("eq", Some(b)) => dc.eq = b,
                        ("order", Some(b)) => dc.order = b,
                        ("frozen", Some(b)) => dc.frozen = b,
                        ("repr", Some(b)) => dc.repr = b,
                        ("kw_only", Some(b)) => dc.kw_only = b,
                        // No observable effect on a native class.
                        ("slots" | "init" | "match_args" | "weakref_slot", Some(true)) | ("slots" | "weakref_slot", Some(false)) => {}
                        (o, _) => {
                            let dsp = self.span(d);
                            self.unsupported(dsp, &format!("`@dataclass({o}=...)`"), "eq, order, frozen, repr, kw_only and slots take a literal True/False");
                        }
                    }
                }
            } else {
                let dsp = self.span(d);
                self.unsupported(dsp, &format!("class decorator `{dname}`"), "only @dataclass is compiled natively");
            }
        }
        // `metaclass=ABCMeta` only makes `@abstractmethod` effective.
        let keywords: Vec<&Value> = arr(st, "keywords").iter().filter(|k| !(k["arg"] == "metaclass" && self.is_abc_name(&k["value"], "ABCMeta"))).collect();
        if !keywords.is_empty() {
            self.unsupported(sp, "class keywords (e.g. `metaclass=`)", "dynamic class creation keeps CPython; put the class in a module that falls back");
        }
        // `ABC` as a base adds nothing but abstract-method checking.
        let bases: Vec<Value> = arr(st, "bases").iter().filter(|b| !self.is_abc_name(b, "ABC")).cloned().collect();
        let bases = bases.as_slice();
        // Each base: a native class, a builtin exception, or `object`.
        let mut native_bases: Vec<ClassId> = Vec::new();
        let mut exc_base: Option<String> = None;
        for b in bases {
            if ty(b) == "Name" && sstr(b, "id") == "object" {
                continue;
            }
            if ty(b) == "Name" && self.class_ids.contains_key(sstr(b, "id")) {
                native_bases.push(self.class_ids[sstr(b, "id")].0);
                continue;
            }
            if ty(b) == "Attribute" {
                if let Some(Binding::Unit(u)) = self.module_binding(&b["value"]) {
                    if let Some(&c) = self.ld.unit_classes[u].get(sstr(b, "attr")) {
                        native_bases.push(c);
                        continue;
                    }
                }
            }
            if ty(b) == "Name" && is_builtin_exception(sstr(b, "id")) && exc_base.is_none() {
                exc_base = Some(sstr(b, "id").to_string());
                continue;
            }
            self.unsupported(sp, &format!("class `{name}` with a base class that is not a natively compiled class"), "keep this class in a module that is imported through CPython");
        }
        let base = match (native_bases.first(), exc_base) {
            (Some(&b), None) => Some(ClassBase::Native(b)),
            (None, Some(e)) => Some(ClassBase::Exception(e)),
            (None, None) => None,
            (Some(&b), Some(e)) => {
                // A native exception class already carries its builtin root.
                if native_bases.iter().any(|&nb| self.exception_root(nb).is_some()) {
                    Some(ClassBase::Native(b))
                } else {
                    self.unsupported(sp, &format!("class `{name}` mixing a builtin exception `{e}` with other bases"), "derive the exception from one class");
                    Some(ClassBase::Native(b))
                }
            }
        };
        let mut methods = Vec::new();
        let mut class_vars: Vec<String> = Vec::new();
        let mut fields: Vec<String> = Vec::new();
        let mut abstracts: Vec<String> = Vec::new();
        let mut enum_kind: Option<String> = None;
        for item in arr(st, "body") {
            if ty(item) == "Assign" && arr(item, "targets").first().is_some_and(|t| sstr(t, "id") == "__pyrock_enum__") {
                enum_kind = item["value"]["value"]["value"].as_str().map(String::from);
                continue;
            }
            let isp = self.span(item);
            match ty(item) {
                "FunctionDef" => {
                    let mname = sstr(item, "name").to_string();
                    let mut kind = MethodKind::Instance;
                    for d in arr(item, "decorator_list") {
                        if self.is_abc_name(d, "abstractmethod") {
                            abstracts.push(mname.clone());
                            continue;
                        }
                        match (ty(d), sstr(d, "id"), sstr(d, "attr")) {
                            ("Name", "staticmethod", _) => kind = MethodKind::Static,
                            ("Name", "classmethod", _) => kind = MethodKind::Class,
                            ("Name", "property", _) => kind = MethodKind::Property,
                            ("Attribute", _, "setter") => kind = MethodKind::Setter,
                            _ => {
                                let dsp = self.span(d);
                                self.unsupported(dsp, "this method decorator", "only @staticmethod, @classmethod, @property and @x.setter are compiled natively");
                            }
                        }
                    }
                    let id = self.ld.alloc_func();
                    let (params, sig) = signature(&item["args"]);
                    let mut anns: Vec<Option<Ty>> = params.iter().map(|p| parse_ann(&p["annotation"])).collect();
                    anns.extend(sig.vararg.iter().chain(&sig.kwarg).map(|_| None));
                    self.ld.func_anns.insert(id, (anns, parse_ann(&item["returns"])));
                    self.ld.func_sigs.insert(id, call_sig(&sig, self.unit));
                    self.hoist_defaults(id, &format!("{name}.{mname}"), &sig, isp);
                    if matches!(kind, MethodKind::Instance | MethodKind::Property | MethodKind::Setter) {
                        match sig.names.first() {
                            Some(self_name) => scan_self_fields(arr(item, "body"), self_name, &mut fields),
                            None => self.err("P303", isp, format!("method `{name}.{mname}` needs a `self` parameter")),
                        }
                    }
                    methods.push(Method { name: mname, func: id, kind });
                }
                "Assign" => {
                    for t in arr(item, "targets") {
                        if ty(t) == "Name" {
                            if !class_vars.iter().any(|v| v == sstr(t, "id")) {
                                class_vars.push(sstr(t, "id").to_string());
                            }
                        } else {
                            self.unsupported(isp, "this class-body assignment", "assign names only");
                        }
                    }
                }
                "AnnAssign" if ty(&item["target"]) == "Name" => {
                    let v = sstr(&item["target"], "id").to_string();
                    if dataclass {
                        // A dataclass field (its __init__ always assigns it).
                        if !fields.contains(&v) {
                            fields.push(v.clone());
                        }
                    } else if !item["value"].is_null() && !class_vars.contains(&v) {
                        class_vars.push(v);
                    }
                }
                "Pass" => {}
                "Expr" if ty(&item["value"]) == "Constant" => {} // docstring
                other => self.unsupported(isp, &format!("`{other}` in a class body"), "classes may contain methods and class attributes"),
            }
        }
        let unit_prefix = if self.unit == 0 { String::new() } else { format!("{}_", self.ld.units[self.unit].name.replace('.', "_")) };
        const RESERVED: &[&str] = &["Box", "Vec", "String", "Option", "Result", "Some", "None", "Ok", "Err", "Rc", "RefCell", "Default", "Clone", "Copy", "Self", "Str"];
        let rust = if RESERVED.contains(&name.as_str()) { format!("{unit_prefix}Py{name}") } else { format!("{unit_prefix}{name}") };
        let qual = if self.unit == 0 { name.clone() } else { format!("{}.{name}", self.ld.units[self.unit].name) };
        self.ld.classes.push(ClassDef { id: cid, name: qual, rust, unit: self.unit, span: sp, base, methods, fields, class_vars, dataclass, abstracts, dc, enum_kind, bases: native_bases });
        self.class_ids.insert(name.clone(), (cid, sp));
        self.bindings.insert(name.clone(), Binding::Class(cid));
        self.ld.unit_classes[self.unit].insert(name, cid);
    }

    /// Pass 2: lower a class's methods; return the class-attribute assignments
    /// (they run at the class statement, in order).
    fn lower_class(&mut self, st: &Value) -> Vec<Stmt> {
        let name = sstr(st, "name");
        let Some(&(cid, sp)) = self.class_ids.get(name) else { return vec![] };
        if sp != self.span(st) {
            return vec![];
        }
        let methods = self.ld.classes[cid as usize].methods.clone();
        let mut out = Vec::new();
        if self.ld.classes[cid as usize].dataclass && !methods.iter().any(|m| m.name == "__init__") {
            self.synthesize_dataclass_init(cid, st);
        }
        let mut mi = 0;
        for item in arr(st, "body") {
            match ty(item) {
                "FunctionDef" => {
                    let m = &methods[mi];
                    mi += 1;
                    let mfunc = m.func;
                    self.cur_fid = None;
                    self.locals = self.module_vars.difference(&self.globals).cloned().collect();
                    self.in_module = true;
                    out.extend(self.default_assigns(mfunc));
                    let m = &methods[mi - 1];
                    let self_name = match m.kind {
                        MethodKind::Static | MethodKind::Class => None,
                        _ => self.ld.func_sigs.get(&m.func).and_then(|s| s.names.first().cloned()),
                    };
                    self.cur_class = Some((cid, self_name));
                    self.locals = HashSet::new();
                    let f = self.lower_function(m.func, item);
                    self.cur_class = None;
                    self.ld.funcs[m.func] = Some(f);
                }
                "Assign" | "AnnAssign" => {
                    if item["value"].is_null() || (ty(item) == "AnnAssign" && self.ld.classes[cid as usize].dataclass) {
                        continue;
                    }
                    if arr(item, "targets").first().is_some_and(|t| sstr(t, "id") == "__pyrock_enum__") {
                        continue;
                    }
                    self.cur_fid = None;
                    self.locals = self.module_vars.difference(&self.globals).cloned().collect();
                    self.in_module = true;
                    let saved_enum = self.in_enum_body;
                    if self.ld.classes[cid as usize].enum_kind.is_some() {
                        self.in_enum_body = Some(cid);
                    }
                    let value = self.lower_expr(&item["value"]);
                    self.in_enum_body = saved_enum;
                    let Some(value) = value else { continue };
                    let isp = self.span(item);
                    let targets: Vec<&Value> = if ty(item) == "Assign" { arr(item, "targets").iter().collect() } else { vec![&item["target"]] };
                    for t in targets {
                        if ty(t) == "Name" {
                            out.push(Stmt { kind: StmtKind::AssignGlobal { unit: self.unit, name: format!("{name}.{}", sstr(t, "id")), value: value.clone() }, span: isp });
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// `@dataclass`: `__init__(self, <fields in order>)` assigning each field,
    /// then calling `__post_init__` if defined.
    fn synthesize_dataclass_init(&mut self, cid: ClassId, st: &Value) {
        let sp = self.span(st);
        let mut names = Vec::new();
        let mut defaults = Vec::new();
        for item in arr(st, "body") {
            if ty(item) != "AnnAssign" || ty(&item["target"]) != "Name" {
                continue;
            }
            let ann = &item["annotation"];
            let is_classvar = (ty(ann) == "Subscript" && (sstr(&ann["value"], "id") == "ClassVar" || sstr(&ann["value"], "attr") == "ClassVar")) || sstr(ann, "id") == "ClassVar";
            if is_classvar {
                continue;
            }
            let name = sstr(&item["target"], "id").to_string();
            let v = &item["value"];
            let default = if v.is_null() {
                None
            } else if ty(v) == "Call" && (sstr(&v["func"], "id") == "field" || sstr(&v["func"], "attr") == "field") {
                let mut d = None;
                for k in arr(v, "keywords") {
                    let vsp = self.span(&k["value"]);
                    match (sstr(k, "arg"), ty(&k["value"]), sstr(&k["value"], "id")) {
                        // A fresh container per call, like default_factory.
                        ("default_factory", "Name", "list") => d = Some(Expr { kind: ExprKind::List(vec![]), span: vsp }),
                        ("default_factory", "Name", "dict") => d = Some(Expr { kind: ExprKind::Dict(vec![]), span: vsp }),
                        ("default_factory", "Name", "set") => d = Some(Expr { kind: ExprKind::Set(vec![]), span: vsp }),
                        ("default", _, _) => d = lower_literal(&k["value"], self.unit),
                        _ => self.unsupported(vsp, "this `field(...)` option", "use `default=` with a literal or `default_factory=list/dict/set`"),
                    }
                }
                d
            } else {
                match lower_literal(v, self.unit) {
                    Some(d) => Some(d),
                    None => {
                        let vsp = self.span(v);
                        self.unsupported(vsp, "a non-literal dataclass default", "use `field(default_factory=...)`");
                        None
                    }
                }
            };
            names.push(name);
            defaults.push(default);
        }
        let id = self.ld.alloc_func();
        let self_name = "self".to_string();
        let mut params = vec![self_name.clone()];
        params.extend(names.iter().cloned());
        let mut sig_defaults = vec![None];
        sig_defaults.extend(defaults);
        // `@dataclass(kw_only=True)`: every field is keyword-only.
        let n_pos = if self.ld.classes[cid as usize].dc.kw_only { 1 } else { params.len() };
        let sig = CallSig { names: params.clone(), n_posonly: 0, n_pos, defaults: sig_defaults, vararg: None, kwarg: None };
        let local = |n: &str| Expr { kind: ExprKind::Local(n.to_string()), span: sp };
        let mut body: Vec<Stmt> = names
            .iter()
            .map(|n| Stmt { kind: StmtKind::SetAttr { obj: local(&self_name), name: n.clone(), value: local(n) }, span: sp })
            .collect();
        if self.method_of(cid, "__post_init__").is_some() {
            let call = Expr {
                kind: ExprKind::CallObj { func: Box::new(Expr { kind: ExprKind::GetAttr { obj: Box::new(local(&self_name)), name: "__post_init__".into() }, span: sp }), args: vec![], kwargs: vec![] },
                span: sp,
            };
            body.push(Stmt { kind: StmtKind::Expr(call), span: sp });
        }
        let cname = self.ld.classes[cid as usize].name.rsplit('.').next().unwrap_or("").to_string();
        let n = params.len();
        self.ld.func_sigs.insert(id, sig.clone());
        self.ld.func_anns.insert(id, (vec![None; n], None));
        self.ld.funcs[id] = Some(Function {
            id,
            name: self.qual(&format!("{cname}.__init__")),
            params: params.clone(),
            locals: params,
            body,
            span: sp,
            is_module: false,
            unit: self.unit,
            param_ann: vec![None; n],
            ret_ann: None,
            class: Some(cid),
            sig,
            parent: None,
            captures: vec![],
            cells: vec![],
            is_async: false,
            is_generator: false,
        });
        self.ld.classes[cid as usize].methods.push(Method { name: "__init__".into(), func: id, kind: MethodKind::Instance });
        let fields = &mut self.ld.classes[cid as usize].fields;
        for nm in names {
            if !fields.contains(&nm) {
                fields.push(nm);
            }
        }
    }

    /// Index (in `scopes`) of the innermost enclosing function that binds `id`.
    fn find_enclosing(&self, id: &str) -> Option<usize> {
        self.scopes.iter().rposition(|(f, locals)| f.is_some() && locals.contains(id))
    }

    /// `id` belongs to `scopes[k]`: it becomes a cell there, and every
    /// function between (and the current one) captures it.
    fn record_capture(&mut self, id: &str, k: usize) {
        let Some(owner) = self.scopes[k].0 else { return };
        let push = |m: &mut HashMap<FuncId, Vec<String>>, f: FuncId| {
            let v = m.entry(f).or_default();
            if !v.iter().any(|x| x == id) {
                v.push(id.to_string());
            }
        };
        push(&mut self.ld.func_cells, owner);
        for j in k + 1..self.scopes.len() {
            if let Some(f) = self.scopes[j].0 {
                push(&mut self.ld.func_captures, f);
            }
        }
        if let Some(cur) = self.cur_fid {
            push(&mut self.ld.func_captures, cur);
        }
    }

    /// Lower a nested function or lambda; returns its closure value.
    fn lower_nested(&mut self, fid: FuncId, st: &Value) -> Option<Expr> {
        let sp = self.span(st);
        let saved_locals = self.locals.clone();
        self.scopes.push((self.cur_fid, saved_locals));
        let saved = (
            std::mem::take(&mut self.locals),
            self.in_module,
            std::mem::take(&mut self.extra_locals),
            self.ret_ann.take(),
            self.cur_fid,
            std::mem::take(&mut self.fn_globals),
            std::mem::take(&mut self.fn_nonlocals),
            self.cur_class.take(),
        );
        if let Some(p) = self.cur_fid {
            self.ld.func_parent.insert(fid, p);
        }
        // Non-literal defaults are evaluated now, in the enclosing scope (saved
        // state), and stored in the closure's environment.
        let (_, jsig) = signature(&st["args"]);
        let mut env_defaults: Vec<(usize, Expr)> = Vec::new();
        {
            let inner = (std::mem::replace(&mut self.locals, saved.0.clone()), self.in_module, self.cur_fid, self.cur_class.take());
            self.in_module = saved.1;
            self.cur_fid = saved.4;
            for (i, d) in jsig.defaults.iter().enumerate() {
                if let Some(d) = d {
                    if !literal_default(d) {
                        if let Some(v) = self.lower_expr(d) {
                            env_defaults.push((i, v));
                        }
                    }
                }
            }
            (self.locals, self.in_module, self.cur_fid, self.cur_class) = inner;
        }
        self.cur_fid = Some(fid);
        let f = self.lower_function(fid, st);
        self.ld.funcs[fid] = Some(f);
        (self.locals, self.in_module, self.extra_locals, self.ret_ann, self.cur_fid, self.fn_globals, self.fn_nonlocals, self.cur_class) = saved;
        self.scopes.pop();
        let mut captures = self.ld.func_captures.get(&fid).cloned().unwrap_or_default();
        let mut defaults = Vec::new();
        for (i, v) in env_defaults {
            let slot = captures.len();
            let hidden = format!("__dflt{slot}");
            captures.push(hidden.clone());
            self.ld.func_captures.entry(fid).or_default().push(hidden);
            let dflt = Some(Expr { kind: ExprKind::EnvDefault(slot), span: v.span });
            if let Some(sig) = self.ld.func_sigs.get_mut(&fid) {
                if let Some(d) = sig.defaults.get_mut(i) {
                    *d = dflt.clone();
                }
            }
            if let Some(Some(f)) = self.ld.funcs.get_mut(fid) {
                if let Some(d) = f.sig.defaults.get_mut(i) {
                    *d = dflt;
                }
            }
            defaults.push(v);
        }
        let captures: Vec<String> = captures.into_iter().take(captures_len(&self.ld.func_captures, fid, defaults.len())).collect();
        Some(Expr { kind: ExprKind::MakeClosure { func: fid, captures, defaults }, span: sp })
    }

    /// `len` -> `lambda __x: len(__x)` for builtins passed as function values.
    fn wrap_builtin_fn(&mut self, e: &Expr, sp: Span) -> Option<Expr> {
        let ExprKind::PyBuiltin(n) = &e.kind else { return None };
        if !matches!(n.as_str(), "len" | "str" | "repr" | "abs" | "int" | "float" | "bool" | "ord" | "chr" | "sum" | "sorted" | "list" | "tuple" | "set" | "min" | "max" | "round") {
            return None;
        }
        let loc = serde_json::json!([sp.line, sp.col, sp.end_line, sp.end_col]);
        let lam = serde_json::json!({
            "_t": "Lambda", "_loc": loc,
            "args": {"_t": "arguments", "posonlyargs": [], "args": [{"_t": "arg", "arg": "__x", "annotation": null, "_loc": loc}],
                     "vararg": null, "kwonlyargs": [], "kw_defaults": [], "kwarg": null, "defaults": []},
            "body": {"_t": "Call", "_loc": loc, "func": {"_t": "Name", "id": n, "_loc": loc}, "args": [{"_t": "Name", "id": "__x", "_loc": loc}], "keywords": []},
        });
        self.lower_expr(&lam)
    }

    /// `Struct.name(...)` (an associated function), or `Struct(...)` (its `new`,
    /// or the field-wise constructor of a C struct).
    fn extern_ctor(&mut self, e: usize, si: usize, name: &str, args: Vec<Expr>, sp: Span) -> Option<Expr> {
        let m = &self.ld.externs[e];
        let sname = m.structs[si].name.clone();
        let want = [format!("{sname}::{name}"), format!("{sname}::__new__")];
        let found = m.funcs.iter().position(|f| f.owner == Some(si) && f.recv.is_none() && (f.name == want[0] || (name == "new" && f.name == want[1])));
        match found {
            Some(func) => Some(Expr { kind: ExprKind::ExternCall { ext: e, func, args }, span: sp }),
            None => {
                self.err("P150", sp, format!("`{}.{sname}` has no associated function `{name}`", m.name));
                None
            }
        }
    }

    /// A fresh synthetic local of the current function.
    fn fresh_local(&mut self) -> String {
        self.tmp += 1;
        let n = format!("__u{}", self.tmp);
        self.locals.insert(n.clone());
        self.extra_locals.push(n.clone());
        n
    }

    /// Several statements in one statement slot.
    fn block_stmt(&self, stmts: Vec<Stmt>, sp: Span) -> Stmt {
        Stmt { kind: StmtKind::If { test: Expr { kind: ExprKind::Bool(true), span: sp }, body: stmts, orelse: vec![] }, span: sp }
    }

    /// Assign `value` to an arbitrary target (name, attribute, subscript,
    /// or a tuple/list pattern with at most one starred name).
    fn lower_target(&mut self, t: &Value, value: Expr, sp: Span) -> Option<Vec<Stmt>> {
        match ty(t) {
            "Name" => {
                let name = sstr(t, "id").to_string();
                if let Some(r) = self.comp_scopes.iter().rev().find_map(|m| m.get(&name)).cloned() {
                    return Some(vec![Stmt { kind: StmtKind::Assign { target: r, value }, span: sp }]);
                }
                Some(vec![Stmt { kind: self.bind_target(name, value), span: sp }])
            }
            "Attribute" | "Subscript" => Some(vec![self.lower_store(t, value, sp)?]),
            "Tuple" | "List" => {
                let elts = arr(t, "elts");
                let stars: Vec<usize> = elts.iter().enumerate().filter(|(_, e)| ty(e) == "Starred").map(|(i, _)| i).collect();
                if stars.len() > 1 {
                    self.err("P001", sp, "SyntaxError: multiple starred expressions in assignment");
                    return None;
                }
                let tmp = self.fresh_local();
                let n = elts.len();
                let unpack = Expr { kind: ExprKind::Unpack { value: Box::new(value), n, star: stars.first().copied() }, span: sp };
                let mut out = vec![Stmt { kind: StmtKind::Assign { target: tmp.clone(), value: unpack }, span: sp }];
                for (i, el) in elts.iter().enumerate() {
                    let item = Expr {
                        kind: ExprKind::Subscript {
                            obj: Box::new(Expr { kind: ExprKind::Local(tmp.clone()), span: sp }),
                            index: Box::new(Expr { kind: ExprKind::Int(i as i64), span: sp }),
                        },
                        span: sp,
                    };
                    let target = if ty(el) == "Starred" { &el["value"] } else { el };
                    out.extend(self.lower_target(target, item, sp)?);
                }
                Some(out)
            }
            other => {
                let tsp = self.span(t);
                self.unsupported(tsp, &format!("assignment to `{other}`"), "assign to names, attributes, subscripts or tuples of those");
                None
            }
        }
    }

    /// `[elt for x in xs if c]`, `{k: v for ...}`, `{e for ...}`, `(e for ...)`.
    /// `(elt for t in it if c ...)`: a hidden generator function called with
    /// the first iterable (evaluated here, in the enclosing scope, as CPython does):
    /// `def <genexpr>(__genexpr_it): for t in __genexpr_it: if c: yield elt`.
    /// A comprehension as a hidden function called with the first iterable
    /// (evaluated here, in the enclosing scope, as CPython does):
    /// * `(elt for t in it if c)`: `def <genexpr>(__genexpr_it): for t in __genexpr_it: if c: yield elt`
    ///   (an async generator if it has `async for` or `await`);
    /// * an async list/set/dict comprehension: `async def <acomp>(__genexpr_it): out = []; async for ...: out.append(elt); return out`,
    ///   awaited in place (awaiting a native coroutine runs it inline).
    fn lower_hidden_comp(&mut self, e: &Value) -> Option<Expr> {
        let sp = self.span(e);
        let loc = &e["_loc"];
        let gens = arr(e, "generators");
        let is_async = gens.iter().any(|g| g["is_async"].as_u64().unwrap_or(0) != 0)
            || contains_await(&e["elt"])
            || contains_await(&e["key"])
            || contains_await(&e["value"])
            || gens.iter().enumerate().any(|(i, g)| (i > 0 && contains_await(&g["iter"])) || arr(g, "ifs").iter().any(contains_await));
        let name = |n: &str| serde_json::json!({"_t": "Name", "id": n, "ctx": {"_t": "Load"}, "_loc": loc});
        let out = "__comp_out";
        let (inner, init, name_s) = match ty(e) {
            "GeneratorExp" => (serde_json::json!({"_t": "Expr", "_loc": loc, "value": {"_t": "Yield", "_loc": loc, "value": e["elt"]}}), None, "<genexpr>"),
            "ListComp" => (
                serde_json::json!({"_t": "Expr", "_loc": loc, "value": {"_t": "Call", "_loc": loc, "func": {"_t": "Attribute", "_loc": loc, "value": name(out), "attr": "append"}, "args": [e["elt"]], "keywords": []}}),
                Some(serde_json::json!({"_t": "List", "_loc": loc, "elts": []})),
                "<listcomp>",
            ),
            "SetComp" => (
                serde_json::json!({"_t": "Expr", "_loc": loc, "value": {"_t": "Call", "_loc": loc, "func": {"_t": "Attribute", "_loc": loc, "value": name(out), "attr": "add"}, "args": [e["elt"]], "keywords": []}}),
                Some(serde_json::json!({"_t": "Call", "_loc": loc, "func": name("set"), "args": [], "keywords": []})),
                "<setcomp>",
            ),
            _ => (
                serde_json::json!({"_t": "Assign", "_loc": loc, "targets": [{"_t": "Subscript", "_loc": loc, "value": name(out), "slice": e["key"], "ctx": {"_t": "Store"}}], "value": e["value"]}),
                Some(serde_json::json!({"_t": "Dict", "_loc": loc, "keys": [], "values": []})),
                "<dictcomp>",
            ),
        };
        let first = self.lower_expr(&self.enum_members_json(&gens[0]["iter"]))?;
        let mut body = serde_json::json!([inner]);
        for (gi, g) in gens.iter().enumerate().rev() {
            for c in arr(g, "ifs").iter().rev() {
                body = serde_json::json!([{"_t": "If", "_loc": loc, "test": c, "body": body, "orelse": []}]);
            }
            let iter = if gi == 0 { name("__genexpr_it") } else { self.enum_members_json(&g["iter"]) };
            let is_async_for = g["is_async"].as_u64().unwrap_or(0) != 0;
            body = serde_json::json!([{"_t": "For", "_loc": loc, "target": g["target"], "iter": iter, "body": body, "orelse": [], "is_async": is_async_for}]);
        }
        if let Some(init) = init {
            let mut stmts = vec![serde_json::json!({"_t": "Assign", "_loc": loc, "targets": [{"_t": "Name", "id": out, "ctx": {"_t": "Store"}, "_loc": loc}], "value": init})];
            stmts.extend(body.as_array().unwrap().iter().cloned());
            stmts.push(serde_json::json!({"_t": "Return", "_loc": loc, "value": name(out)}));
            body = serde_json::Value::Array(stmts);
        }
        let mut def = serde_json::json!({
            "_t": "FunctionDef", "name": name_s, "decorator_list": [], "returns": null, "_loc": loc, "body": body,
            "args": {"_t": "arguments", "posonlyargs": [], "args": [{"_t": "arg", "arg": "__genexpr_it", "annotation": null, "_loc": loc}],
                     "vararg": null, "kwonlyargs": [], "kw_defaults": [], "kwarg": null, "defaults": []},
        });
        if is_async {
            def["is_async"] = serde_json::json!(true);
        }
        let fid = self.ld.alloc_func();
        let (params, sig) = signature(&def["args"]);
        self.ld.func_anns.insert(fid, (params.iter().map(|_| None).collect(), None));
        self.ld.func_sigs.insert(fid, call_sig(&sig, self.unit));
        let func = self.lower_nested(fid, &def)?;
        let call = Expr { kind: ExprKind::CallObj { func: Box::new(func), args: vec![first], kwargs: vec![] }, span: sp };
        if is_async && init_is_some(ty(e)) {
            return Some(Expr { kind: ExprKind::Await(Box::new(call)), span: sp });
        }
        Some(call)
    }

    fn lower_comp(&mut self, e: &Value) -> Option<Expr> {
        let sp = self.span(e);
        // Generator expressions, and comprehensions with `async for` or
        // `await`, are hidden functions.
        let any_async = arr(e, "generators").iter().any(|g| g["is_async"].as_u64().unwrap_or(0) != 0)
            || ["elt", "key", "value"].iter().any(|k| contains_await(&e[*k]))
            || arr(e, "generators").iter().any(|g| contains_await(&g["iter"]) || arr(g, "ifs").iter().any(contains_await));
        if ty(e) == "GeneratorExp" || any_async {
            return self.lower_hidden_comp(e);
        }
        let kind = match ty(e) {
            "ListComp" => CompKind::List,
            "SetComp" => CompKind::Set,
            "DictComp" => CompKind::Dict,
            _ => CompKind::Gen,
        };
        let mut gens = Vec::new();
        let pushed = self.comp_scopes.len();
        for (gi, g) in arr(e, "generators").iter().enumerate() {
            if g["is_async"].as_u64().unwrap_or(0) != 0 {
                self.unsupported(sp, "async comprehension", "use a loop");
                return None;
            }
            // The first iterable is evaluated in the enclosing scope.
            let iter = if gi == 0 {
                let saved = self.comp_scopes.split_off(pushed);
                let saved_owner = self.comp_owner.split_off(pushed);
                let it = self.lower_expr(&self.enum_members_json(&g["iter"]));
                self.comp_scopes.extend(saved);
                self.comp_owner.extend(saved_owner);
                it?
            } else {
                self.lower_expr(&self.enum_members_json(&g["iter"]))?
            };
            let mut names = HashSet::new();
            scan_names(&g["target"], &mut names);
            let mut scope = HashMap::new();
            for n in names {
                self.tmp += 1;
                let r = format!("__c{}_{n}", self.tmp);
                self.locals.insert(r.clone());
                self.extra_locals.push(r.clone());
                scope.insert(n, r);
            }
            self.comp_scopes.push(scope);
            self.comp_owner.push(self.cur_fid);
            let var = self.fresh_local();
            let read = Expr { kind: ExprKind::Local(var.clone()), span: sp };
            let unpack = self.lower_target(&g["target"], read, sp)?;
            let mut ifs = Vec::new();
            for c in arr(g, "ifs") {
                ifs.push(self.lower_expr(c)?);
            }
            gens.push(CompFor { var, iter, unpack, ifs });
        }
        let r = (|| {
            if kind == CompKind::Dict {
                let k = self.lower_expr(&e["key"])?;
                let v = self.lower_expr(&e["value"])?;
                Some((k, Some(Box::new(v))))
            } else {
                Some((self.lower_expr(&e["elt"])?, None))
            }
        })();
        self.comp_scopes.truncate(pushed);
        self.comp_owner.truncate(pushed);
        let (elt, val) = r?;
        Some(Expr { kind: ExprKind::Comp { kind, elt: Box::new(elt), val, gens }, span: sp })
    }

    /// `name = value` for a local or a global cell.
    fn bind_target(&mut self, name: String, value: Expr) -> StmtKind {
        if self.fn_nonlocals.contains(&name) {
            return StmtKind::Assign { target: name, value };
        }
        if !self.locals.contains(&name) && (self.fn_globals.contains(&name) || self.globals.contains(&name)) {
            StmtKind::AssignGlobal { unit: self.unit, name, value }
        } else {
            StmtKind::Assign { target: name, value }
        }
    }

    /// A loop / `with` / `except` target that is a global: bind a synthetic
    /// local and copy it into the global cell at the start of the body.
    fn local_target(&mut self, name: String, sp: Span) -> (String, Option<Stmt>) {
        if self.locals.contains(&name) || !(self.fn_globals.contains(&name) || self.globals.contains(&name)) {
            return (name, None);
        }
        let tmp = format!("__g_{name}");
        if !self.locals.contains(&tmp) {
            self.locals.insert(tmp.clone());
            self.extra_locals.push(tmp.clone());
        }
        let set = StmtKind::AssignGlobal { unit: self.unit, name, value: Expr { kind: ExprKind::Local(tmp.clone()), span: sp } };
        (tmp, Some(Stmt { kind: set, span: sp }))
    }

    /// `name` refers to a builtin exception class (not shadowed by anything local).
    fn builtin_exc(&self, name: &str) -> bool {
        is_builtin_exception(name) && !self.locals.contains(name) && !self.bindings.contains_key(name) && !self.module_vars.contains(name)
    }

    /// The type in an `except` clause.
    fn exc_type(&mut self, t: &Value) -> Option<ExcType> {
        if ty(t) == "Name" && self.builtin_exc(sstr(t, "id")) {
            return Some(ExcType::Builtin(sstr(t, "id").to_string()));
        }
        if ty(t) == "Name" && !self.locals.contains(sstr(t, "id")) {
            if let Some(Binding::Class(c)) = self.bindings.get(sstr(t, "id")).cloned() {
                return Some(ExcType::Native(c));
            }
        }
        Some(ExcType::Obj(self.lower_expr(t)?))
    }

    /// If `v` names a natively compiled module or extern, return its binding.
    fn module_binding(&self, v: &Value) -> Option<Binding> {
        // `pkg.sub` once `pkg.sub` has been imported.
        if ty(v) == "Attribute" {
            if let Some(Binding::Unit(u)) = self.module_binding(&v["value"]) {
                if let Some(&s) = self.ld.unit_subs[u].get(sstr(v, "attr")) {
                    return Some(Binding::Unit(s));
                }
            }
            return None;
        }
        if ty(v) != "Name" || self.locals.contains(sstr(v, "id")) {
            return None;
        }
        match self.bindings.get(sstr(v, "id")) {
            Some(b @ (Binding::Unit(_) | Binding::Extern(_))) => Some(b.clone()),
            _ => None,
        }
    }

    fn lower_name(&mut self, id: &str, sp: Span) -> Option<Expr> {
        if let Some(k) = self.comp_scopes.iter().rposition(|m| m.contains_key(id)) {
            let r = self.comp_scopes[k][id].clone();
            // A renamed local of an enclosing function: captured.
            if self.comp_owner[k] != self.cur_fid && !self.locals.contains(&r) {
                if let Some(owner) = self.comp_owner[k] {
                    if let Some(si) = self.scopes.iter().position(|s| s.0 == Some(owner)) {
                        self.record_capture(&r, si);
                    }
                }
            }
            return Some(Expr { kind: ExprKind::Local(r), span: sp });
        }
        if self.fn_nonlocals.contains(id) {
            return Some(Expr { kind: ExprKind::Local(id.to_string()), span: sp });
        }
        if !self.locals.contains(id) {
            if let Some(k) = self.find_enclosing(id) {
                self.record_capture(id, k);
                return Some(Expr { kind: ExprKind::Local(id.to_string()), span: sp });
            }
        }
        let kind = if self.locals.contains(id) {
            ExprKind::Local(id.to_string())
        } else if self.fn_globals.contains(id) || self.globals.contains(id) {
            ExprKind::Global { unit: self.unit, name: id.to_string() }
        } else if id == "__name__" {
            ExprKind::Str(if self.unit == 0 { "__main__".into() } else { self.ld.units[self.unit].name.clone() })
        } else if id == "__file__" {
            // CPython: the path as given for the entry script, absolute for imports.
            ExprKind::Str(self.ld.units[self.unit].path.clone())
        } else if let Some(b) = self.bindings.get(id).cloned() {
            match b {
                Binding::Slot(s) => {
                    self.used_slots.insert(s);
                    ExprKind::Slot(s)
                }
                Binding::Class(c) => ExprKind::ClassRef(c),
                Binding::Func(f) => ExprKind::MakeClosure { func: f, captures: vec![], defaults: vec![] },
                _ => {
                    self.unsupported(sp, &format!("using `{id}` as a first-class value"), "call it directly");
                    return None;
                }
            }
        } else if self.module_vars.contains(id) {
            self.diags.push(
                Diagnostic::error("P140", sp, format!("read of module-level variable `{id}` inside a function"))
                    .note("module globals may be rebound at runtime; pass the value as a parameter instead"),
            );
            return None;
        } else if self.ld.builtins.contains(id) {
            ExprKind::PyBuiltin(id.to_string())
        } else {
            self.err("P143", sp, format!("name `{id}` is not defined"));
            return None;
        };
        Some(Expr { kind, span: sp })
    }

    fn lower_call(&mut self, e: &Value) -> Option<Expr> {
        // `list(Color)`, `len(Color)`, `sorted(Color)`, ...: over the members.
        let callee = if ty(&e["func"]) == "Name" { sstr(&e["func"], "id") } else { "" };
        if matches!(callee, "list" | "len" | "sorted" | "tuple" | "set" | "enumerate" | "reversed" | "iter" | "any" | "all" | "min" | "max" | "zip" | "map" | "filter")
            && arr(e, "args").iter().any(|a| self.enum_class_of(a).is_some())
        {
            let mut e2 = e.clone();
            let args: Vec<Value> = arr(e, "args").iter().map(|a| self.enum_members_json(a)).collect();
            e2["args"] = Value::Array(args);
            return self.lower_call(&e2);
        }
        let sp = self.span(e);
        let f = &e["func"];
        // Builtins that read the caller's Python frame: compiled code has none,
        // so CPython would fail (`globals()` even returns NULL without an
        // exception). Reject them; an imported module using them is then
        // imported through CPython as a whole (P220), with exact semantics.
        if ty(f) == "Name" && !self.locals.contains(sstr(f, "id")) && !self.bindings.contains_key(sstr(f, "id")) {
            let id = sstr(f, "id");
            let n_args = arr(e, "args").len();
            let has_ns_kw = arr(e, "keywords").iter().any(|k| matches!(k["arg"].as_str(), Some("globals") | None));
            let frame_reader = match id {
                "globals" | "locals" => true,
                "vars" | "dir" => n_args == 0,
                "eval" | "exec" => n_args < 2 && !has_ns_kw,
                _ => false,
            };
            if frame_reader && self.ld.builtins.contains(id) {
                let note = if matches!(id, "eval" | "exec") {
                    "compiled code has no Python namespace for it to use; pass one explicitly, e.g. `eval(src, {})` or `exec(src, ns)`"
                } else {
                    "compiled code has no Python frame or namespace to read; keep this code in a module imported through CPython"
                };
                self.unsupported(sp, &format!("`{id}()` without an explicit namespace"), note);
                return None;
            }
        }
        let mut args = Vec::new();
        for a in arr(e, "args") {
            if ty(a) == "Starred" {
                let asp = self.span(a);
                args.push(Expr { kind: ExprKind::Starred(Box::new(self.lower_expr(&a["value"])?)), span: asp });
            } else {
                args.push(self.lower_expr(a)?);
            }
        }
        let mut kwargs = Vec::new();
        for k in arr(e, "keywords") {
            let name = k["arg"].as_str().unwrap_or("**").to_string();
            kwargs.push((name, self.lower_expr(&k["value"])?));
        }
        let starred = args.iter().any(|a| matches!(a.kind, ExprKind::Starred(_))) || kwargs.iter().any(|(k, _)| k == "**");
        // `key=len`, `map(str, xs)`: a builtin used as a function value is a
        // one-argument lambda, so it runs natively.
        let callee_name = if ty(f) == "Name" { sstr(f, "id") } else if ty(f) == "Attribute" { sstr(f, "attr") } else { "" };
        let mut args = args;
        let mut kwargs = kwargs;
        if matches!(callee_name, "sorted" | "min" | "max" | "sort" | "map" | "filter") {
            for (k, v) in kwargs.iter_mut() {
                if k == "key" {
                    if let Some(w) = self.wrap_builtin_fn(v, sp) {
                        *v = w;
                    }
                }
            }
            if matches!(callee_name, "map" | "filter") {
                if let Some(first) = args.first_mut() {
                    if let Some(w) = self.wrap_builtin_fn(first, sp) {
                        *first = w;
                    }
                }
            }
        }
        let native_only = |me: &mut Self, what: &str| -> Option<()> {
            if !kwargs_is_empty(&kwargs) {
                me.unsupported(sp, &format!("keyword arguments to {what}"), "natively compiled functions take positional arguments only");
                return None;
            }
            Some(())
        };

        // Direct calls by name.
        if ty(f) == "Name" && !self.locals.contains(sstr(f, "id")) {
            let id = sstr(f, "id");
            match self.bindings.get(id).cloned() {
                Some(Binding::Func(fid)) => {
                    if self.in_module {
                        if let Some(&(_, def_sp)) = self.func_ids.get(id) {
                            if def_sp.line > sp.line {
                                self.err("P144", sp, format!("`{id}` is called before its definition (NameError at runtime)"));
                                return None;
                            }
                        }
                    }
                    let (args, order) = self.bind_args(fid, id, args, kwargs, sp)?;
                    let args = self.annotate_args(fid, args);
                    return Some(Expr { kind: ExprKind::Call { callee: Callee::Func(fid), args, order }, span: sp });
                }
                Some(Binding::ExternFn(ext, func)) => {
                    native_only(self, &format!("`{id}`"))?;
                    if starred {
                        self.unsupported(sp, &format!("`*`/`**` arguments to the Rust/C function `{id}`"), "pass arguments explicitly");
                        return None;
                    }
                    return Some(Expr { kind: ExprKind::ExternCall { ext, func, args }, span: sp });
                }
                Some(Binding::Unit(_) | Binding::Extern(_)) => {
                    self.unsupported(sp, &format!("calling module `{id}`"), "modules are not callable");
                    return None;
                }
                Some(Binding::ExternStruct(e, si)) => return self.extern_ctor(e, si, "new", args, sp),
                Some(Binding::Class(c)) => {
                    if self.in_module {
                        if let Some(&(_, def_sp)) = self.class_ids.get(id) {
                            if def_sp.line > sp.line {
                                self.err("P144", sp, format!("`{id}` is used before its class statement (NameError at runtime)"));
                                return None;
                            }
                        }
                    }
                    if self.ld.classes[c as usize].enum_kind.is_some() && self.in_enum_body != Some(c) {
                        // `Color(1)`: the member with that value.
                        return self.lower_class_attr_call(c, "_pyrock_lookup_", args, kwargs, sp);
                    }
                    return Some(Expr { kind: ExprKind::New { class: c, args, kwargs }, span: sp });
                }
                Some(Binding::Slot(_)) | None => {}
            }
            if self.builtin_exc(id) && kwargs.is_empty() && args.len() <= 1 && !starred {
                return Some(Expr { kind: ExprKind::NewExc { kind: id.to_string(), args }, span: sp });
            }
            if !self.bindings.contains_key(id) && !self.module_vars.contains(id) {
                if let Some(b) = Builtin::from_name(id).filter(|b| !starred || *b == Builtin::Print) {
                    if kwargs.is_empty() {
                        return Some(Expr { kind: ExprKind::Call { callee: Callee::Builtin(b), args, order: vec![] }, span: sp });
                    }
                    // print(..., sep=, end=, flush=) stays native; file= uses CPython.
                    if b == Builtin::Print && kwargs.iter().all(|(k, _)| matches!(k.as_str(), "sep" | "end" | "flush")) {
                        let mut args = args;
                        for (k, default) in [("sep", " "), ("end", "\n")] {
                            let v = kwargs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
                            args.push(match v {
                                Some(Expr { kind: ExprKind::NoneLit, .. }) | None => Expr { kind: ExprKind::Str(default.into()), span: sp },
                                Some(v) => v,
                            });
                        }
                        let flush = kwargs.iter().find(|(n, _)| n == "flush").map(|(_, v)| v.clone());
                        args.push(flush.unwrap_or(Expr { kind: ExprKind::Bool(false), span: sp }));
                        return Some(Expr { kind: ExprKind::Call { callee: Callee::Builtin(Builtin::PrintKw), args, order: vec![] }, span: sp });
                    }
                    // e.g. print(x, end=""): use the real CPython builtin.
                }
            }
        }

        // super().method(...) inside a method.
        if ty(f) == "Attribute" && ty(&f["value"]) == "Call" && ty(&f["value"]["func"]) == "Name" && f["value"]["func"]["id"] == "super" && arr(&f["value"], "args").is_empty() {
            return self.lower_super_call(sstr(f, "attr"), args, kwargs, sp);
        }
        // Associated functions of Rust/C structs: shapes.Counter.new(...), Counter.new(...).
        if ty(f) == "Attribute" {
            let target = match ty(&f["value"]) {
                "Name" if !self.locals.contains(sstr(&f["value"], "id")) => match self.bindings.get(sstr(&f["value"], "id")).cloned() {
                    Some(Binding::ExternStruct(e, si)) => Some((e, si)),
                    _ => None,
                },
                "Attribute" => match self.module_binding(&f["value"]["value"]) {
                    Some(Binding::Extern(e)) if e != usize::MAX => self.ld.externs[e].structs.iter().position(|st| st.name == sstr(&f["value"], "attr")).map(|si| (e, si)),
                    _ => None,
                },
                _ => None,
            };
            if let Some((e, si)) = target {
                return self.extern_ctor(e, si, sstr(f, "attr"), args, sp);
            }
        }
        // Class attribute calls: Point.from_pair(...), Point.norm(p).
        if ty(f) == "Attribute" && ty(&f["value"]) == "Name" && !self.locals.contains(sstr(&f["value"], "id")) {
            if let Some(Binding::Class(c)) = self.bindings.get(sstr(&f["value"], "id")).cloned() {
                return self.lower_class_attr_call(c, sstr(f, "attr"), args, kwargs, sp);
            }
        }
        // Module attribute calls: helpers.f(...), mymath.dot(...).
        if ty(f) == "Attribute" {
            if let Some(b) = self.module_binding(&f["value"]) {
                let attr = sstr(f, "attr");
                let modname = sstr(&f["value"], "id").to_string();
                match b {
                    Binding::Unit(u) if self.ld.unit_classes[u].contains_key(attr) => {
                        let c = self.ld.unit_classes[u][attr];
                        if self.ld.classes[c as usize].enum_kind.is_some() {
                            return self.lower_class_attr_call(c, "_pyrock_lookup_", args, kwargs, sp);
                        }
                        return Some(Expr { kind: ExprKind::New { class: c, args, kwargs }, span: sp });
                    }
                    Binding::Unit(u) => match self.ld.unit_funcs[u].get(attr).copied() {
                        Some(fid) => {
                            let (args, order) = self.bind_args(fid, &format!("{modname}.{attr}"), args, kwargs, sp)?;
                            let args = self.annotate_args(fid, args);
                            return Some(Expr { kind: ExprKind::Call { callee: Callee::Func(fid), args, order }, span: sp });
                        }
                        None if self.ld.unit_bindings[u].contains_key(attr) || self.ld.unit_subs[u].contains_key(attr) => {
                            // Re-exported: call it as if the name were bound here.
                            let call = serde_json::json!({"_t": "Call", "func": self.reexport_name(u, attr, &f["_loc"]), "args": e["args"], "keywords": e["keywords"], "_loc": e["_loc"]});
                            return self.lower_call(&call);
                        }
                        None => {
                            self.err("P140", sp, format!("`{modname}.{attr}` is not a function of the natively compiled module `{modname}`"));
                            return None;
                        }
                    },
                    Binding::Extern(ext) => {
                        if ext == usize::MAX {
                            return None;
                        }
                        native_only(self, &format!("`{modname}.{attr}`"))?;
                        if starred {
                            self.unsupported(sp, &format!("`*`/`**` arguments to the Rust/C function `{modname}.{attr}`"), "pass arguments explicitly");
                            return None;
                        }
                        if let Some(si) = self.ld.externs[ext].structs.iter().position(|st| st.name == attr) {
                            return self.extern_ctor(ext, si, "new", args, sp);
                        }
                        return match self.ld.externs[ext].funcs.iter().position(|x| x.name == attr) {
                            Some(func) => Some(Expr { kind: ExprKind::ExternCall { ext, func, args }, span: sp }),
                            None => {
                                self.extern_missing(ext, attr, sp);
                                None
                            }
                        };
                    }
                    _ => {}
                }
            }
        }

        // Everything else is a call of a Python object.
        let func = self.lower_expr(f)?;
        Some(Expr { kind: ExprKind::CallObj { func: Box::new(func), args, kwargs }, span: sp })
    }

    /// Place positional and keyword arguments and defaults into parameter
    /// order. Returns the arguments and their evaluation order (empty when
    /// it is already parameter order).
    fn bind_args(&mut self, fid: FuncId, name: &str, args: Vec<Expr>, kwargs: Vec<(String, Expr)>, sp: Span) -> Option<(Vec<Expr>, Vec<usize>)> {
        let sig = self.ld.func_sigs.get(&fid).cloned().unwrap_or_default();
        match bind_call(&sig, name, args, kwargs, sp) {
            Ok(r) => Some(r),
            Err(BindError::TypeError(m)) => {
                self.err("P303", sp, m);
                None
            }
            Err(BindError::Unsupported(m)) => {
                self.diags.push(Diagnostic::error("P100", sp, format!("{m} is not supported yet")));
                None
            }
        }
    }

    /// Arguments to annotated parameters are checked/unboxed at the call.
    fn annotate_args(&self, fid: FuncId, args: Vec<Expr>) -> Vec<Expr> {
        let anns = self.ld.func_anns.get(&fid).map(|a| a.0.clone()).unwrap_or_default();
        let anns: Vec<Option<Ty>> = anns.into_iter().chain(std::iter::repeat(None)).take(args.len()).collect();
        args.into_iter().enumerate().map(|(i, a)| annotated(anns.get(i).copied().flatten(), a)).collect()
    }

    /// `f"a{x}b"` lowers to `"a" + str(x) + "b"`; format specs and `!r`
    /// conversions use CPython's `format`/`repr`.
    fn lower_fstring(&mut self, e: &Value) -> Option<Expr> {
        let sp = self.span(e);
        let str_call = |v: Expr, sp: Span| Expr { kind: ExprKind::Call { callee: Callee::Builtin(Builtin::Str), args: vec![v], order: vec![] }, span: sp };
        let mut acc: Option<Expr> = None;
        for part in arr(e, "values") {
            let psp = self.span(part);
            let piece = match ty(part) {
                "Constant" => Expr { kind: ExprKind::Str(part["value"]["value"].as_str().unwrap_or("").to_string()), span: sp },
                "FormattedValue" => {
                    let conv = part["conversion"].as_i64().unwrap_or(-1);
                    let mut v = self.lower_expr(&part["value"])?;
                    if conv == 'r' as i64 || conv == 'a' as i64 {
                        let f = if conv == 'r' as i64 { "repr" } else { "ascii" };
                        v = self.call_obj(self.py_builtin(f, psp), vec![v], psp);
                    }
                    if !part["format_spec"].is_null() {
                        let spec = self.lower_fstring(&part["format_spec"])?;
                        v = self.call_obj(self.py_builtin("format", psp), vec![v, spec], psp);
                    }
                    str_call(v, psp)
                }
                _ => return None,
            };
            acc = Some(match acc {
                None => piece,
                Some(l) => Expr { kind: ExprKind::BinOp { op: BinOp::Add, left: Box::new(l), right: Box::new(piece) }, span: sp },
            });
        }
        Some(acc.unwrap_or(Expr { kind: ExprKind::Str(String::new()), span: sp }))
    }
}

fn kwargs_is_empty(k: &[(String, Expr)]) -> bool {
    k.is_empty()
}

/// Whether a statement (of a function body) contains `yield` / `yield from`
/// outside nested functions and classes.
fn contains_yield(v: &Value) -> bool {
    match v {
        Value::Object(m) => match m.get("_t").and_then(|t| t.as_str()) {
            Some("Yield" | "YieldFrom") => true,
            Some("FunctionDef" | "ClassDef" | "Lambda") => false,
            _ => m.iter().any(|(k, x)| k != "_t" && contains_yield(x)),
        },
        Value::Array(xs) => xs.iter().any(contains_yield),
        _ => false,
    }
}

fn init_is_some(t: &str) -> bool {
    t != "GeneratorExp"
}

/// Whether an expression contains `await` outside nested functions.
fn contains_await(v: &Value) -> bool {
    match v {
        Value::Object(m) => match m.get("_t").and_then(|t| t.as_str()) {
            Some("Await") => true,
            Some("FunctionDef" | "ClassDef" | "Lambda") => false,
            _ => m.iter().any(|(k, x)| k != "_t" && contains_await(x)),
        },
        Value::Array(xs) => xs.iter().any(contains_await),
        _ => false,
    }
}
