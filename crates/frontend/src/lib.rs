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

use pyfast_ir::*;
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

/// Defaults are evaluated once, at `def` time, in CPython. pyfast inlines
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
    diags: Vec<Diagnostic>,
}

/// Modules whose imports only matter to type checkers.
const ANNOTATION_ONLY_MODULES: &[&str] = &["typing", "typing_extensions", "__future__", "dataclasses"];

/// Map an annotation to a type pyfast can use. Anything else (generics,
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
                collect_bindings(arr(st, "body"), out, top);
                collect_bindings(arr(st, "orelse"), out, top);
            }
            "If" | "While" => {
                collect_bindings(arr(st, "body"), out, top);
                collect_bindings(arr(st, "orelse"), out, top);
            }
            "Try" => {
                collect_bindings(arr(st, "body"), out, top);
                for h in arr(st, "handlers") {
                    if let Some(n) = h["name"].as_str() {
                        push(n, out);
                    }
                    collect_bindings(arr(h, "body"), out, top);
                }
                collect_bindings(arr(st, "orelse"), out, top);
                collect_bindings(arr(st, "finalbody"), out, top);
            }
            "With" => {
                for item in arr(st, "items") {
                    target(&item["optional_vars"], out);
                }
                collect_bindings(arr(st, "body"), out, top);
            }
            "FunctionDef" if !top => push(sstr(st, "name"), out),
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
        // Pass 1a': discover classes (in order: bases must be defined earlier).
        for st in body {
            if ty(st) == "ClassDef" {
                self.declare_class(st);
            }
        }
        // Pass 1b: resolve imports (they are visible inside functions).
        for st in body {
            match ty(st) {
                "Import" | "ImportFrom" => self.resolve_import(st),
                _ => self.reject_nested_imports(st),
            }
        }
        let mut module_locals = Vec::new();
        collect_bindings(body, &mut module_locals, true);
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
        for st in body.iter().filter(|st| ty(st) == "FunctionDef").chain(methods) {
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

    fn reject_nested_imports(&mut self, st: &Value) {
        for h in arr(st, "handlers") {
            self.reject_nested_imports(h);
        }
        for key in ["body", "orelse", "finalbody"] {
            for s in arr(st, key) {
                if matches!(ty(s), "Import" | "ImportFrom") {
                    let sp = self.span(s);
                    self.unsupported(sp, "import inside a function or block", "move imports to module level");
                } else {
                    self.reject_nested_imports(s);
                }
            }
        }
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
            if st["level"].as_u64().unwrap_or(0) > 0 {
                self.unsupported(sp, "relative import", "use an absolute import next to the entry file");
                return;
            }
            let module = sstr(st, "module").to_string();
            let target = self.resolve_local(&module, sp);
            if let Some(Binding::Unit(u)) = target {
                actions.push(ImportAction::Unit(self.ld.units[u].init));
            }
            for alias in arr(st, "names") {
                let attr = sstr(alias, "name").to_string();
                let bind = alias["asname"].as_str().unwrap_or(&attr).to_string();
                if attr == "*" {
                    self.unsupported(sp, "`from ... import *`", "import the names you use explicitly");
                    continue;
                }
                match target {
                    Some(Binding::Unit(u)) => match self.ld.unit_funcs[u].get(&attr) {
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
        for (n, d) in sig.names.iter().zip(&sig.defaults) {
            if let Some(d) = d {
                if !literal_default(d) && self.scopes.is_empty() {
                    let dsp = self.span(d);
                    self.diags.push(
                        Diagnostic::error("P101", dsp, format!("default value of `{n}` in `{name}` must be a literal"))
                            .note("CPython evaluates defaults once, at `def` time; pyfast inlines them at each call. Use `None` and assign in the body"),
                    );
                }
            }
        }
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
        let body = self.lower_block(arr(st, "body"));
        self.ret_ann = None;
        self.fn_globals.clear();
        locals.extend(std::mem::take(&mut self.extra_locals));
        let sig = self.ld.func_sigs.get(&id).cloned().unwrap_or_default();
        let class = self.cur_class.as_ref().map(|c| c.0);
        let is_async = st["is_async"].as_bool().unwrap_or(false);
        let is_generator = arr(st, "body").iter().any(contains_yield);
        if is_async && is_generator {
            self.unsupported(sp, "async generator", "collect the items in a list, or write a class with `__aiter__`/`__anext__`");
        }
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
            // `yield from it` (as a statement): `for x in it: yield x`.
            "Expr" if ty(&st["value"]) == "YieldFrom" => {
                let iter = self.lower_expr(&st["value"]["value"])?;
                let var = self.fresh_local();
                let item = Expr { kind: ExprKind::Local(var.clone()), span: sp };
                let body = vec![Stmt { kind: StmtKind::Expr(Expr { kind: ExprKind::Yield(Some(Box::new(item))), span: sp }), span: sp }];
                StmtKind::ForIter { var, iter, body, is_async: false }
            }
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
                let value = annotated(parse_ann(&st["annotation"]), self.lower_expr(&st["value"])?);
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
            "Import" | "ImportFrom" => {
                let actions = self.import_actions.remove(&sp).unwrap_or_default();
                let mut stmts: Vec<Stmt> = actions.into_iter().map(|a| Stmt { kind: StmtKind::Import(a), span: sp }).collect();
                for (bind, u, name) in self.var_imports.remove(&sp).unwrap_or_default() {
                    let value = Expr { kind: ExprKind::Global { unit: u, name }, span: sp };
                    stmts.push(Stmt { kind: StmtKind::AssignGlobal { unit: self.unit, name: bind, value }, span: sp });
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
                        _ => {
                            self.unsupported(tsp, "`del` of a name", "rebind it instead (e.g. to None)");
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
        let it = &st["iter"];
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
                    rest.push((op, self.lower_expr(c)?));
                }
                ExprKind::Compare { first: Box::new(first), rest }
            }
            "BoolOp" => {
                let op = if ty(&e["op"]) == "And" { BoolOpKind::And } else { BoolOpKind::Or };
                ExprKind::BoolOp { op, values: self.lower_exprs(arr(e, "values"))? }
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
        let mut cur = Some(c);
        while let Some(k) = cur {
            let cd = &self.ld.classes[k as usize];
            if cd.class_vars.iter().any(|v| v == attr) {
                let short = cd.name.rsplit('.').next().unwrap_or("");
                return Some((cd.unit, format!("{short}.{attr}")));
            }
            cur = match &cd.base {
                Some(ClassBase::Native(b)) => Some(*b),
                _ => None,
            };
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
        let mut cur = Some(c);
        while let Some(k) = cur {
            let cd = &self.ld.classes[k as usize];
            if let Some(m) = cd.methods.iter().find(|m| m.name == name) {
                return Some(m.clone());
            }
            cur = match &cd.base {
                Some(ClassBase::Native(b)) => Some(*b),
                _ => None,
            };
        }
        None
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
        let base = self.ld.classes[c as usize].base.clone();
        match base {
            Some(ClassBase::Native(b)) => {
                let Some(m) = self.method_of(b, name) else {
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
                let mut args = args;
                match m.kind {
                    MethodKind::Instance | MethodKind::Property | MethodKind::Setter => {
                        let Some(sn) = self_name else {
                            self.err("P301", sp, "super() in a method without `self`");
                            return None;
                        };
                        args.insert(0, Expr { kind: ExprKind::Local(sn), span: sp });
                    }
                    MethodKind::Class => args.insert(0, Expr { kind: ExprKind::ClassRef(c), span: sp }),
                    MethodKind::Static => {}
                }
                let (args, order) = self.bind_args(m.func, &format!("super().{name}"), args, kwargs, sp)?;
                Some(Expr { kind: ExprKind::Call { callee: Callee::Func(m.func), args, order }, span: sp })
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
        let mut cur = Some(c);
        while let Some(k) = cur {
            match &self.ld.classes[k as usize].base {
                Some(ClassBase::Exception(e)) => return Some(e.clone()),
                Some(ClassBase::Native(b)) => cur = Some(*b),
                None => return None,
            }
        }
        None
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
        for d in arr(st, "decorator_list") {
            let target = if ty(d) == "Call" { &d["func"] } else { d };
            let dname = if ty(target) == "Name" { sstr(target, "id") } else { sstr(target, "attr") };
            if dname == "dataclass" {
                dataclass = true;
                if ty(d) == "Call" && !arr(d, "keywords").is_empty() {
                    let dsp = self.span(d);
                    self.unsupported(dsp, "`@dataclass(...)` options", "use plain `@dataclass`");
                }
            } else {
                let dsp = self.span(d);
                self.unsupported(dsp, &format!("class decorator `{dname}`"), "only @dataclass is compiled natively");
            }
        }
        if !arr(st, "keywords").is_empty() {
            self.unsupported(sp, "class keywords (e.g. `metaclass=`)", "dynamic class creation keeps CPython; put the class in a module that falls back");
        }
        let bases = arr(st, "bases");
        let base = match bases {
            [] => None,
            [b] if ty(b) == "Name" && sstr(b, "id") == "object" => None,
            [b] if ty(b) == "Name" && self.class_ids.contains_key(sstr(b, "id")) => Some(ClassBase::Native(self.class_ids[sstr(b, "id")].0)),
            [b] if ty(b) == "Name" && is_builtin_exception(sstr(b, "id")) => Some(ClassBase::Exception(sstr(b, "id").to_string())),
            [b] if ty(b) == "Attribute" && self.module_binding(&b["value"]).is_some_and(|m| matches!(m, Binding::Unit(u) if self.ld.unit_classes[u].contains_key(sstr(b, "attr")))) => {
                let Some(Binding::Unit(u)) = self.module_binding(&b["value"]) else { unreachable!() };
                Some(ClassBase::Native(self.ld.unit_classes[u][sstr(b, "attr")]))
            }
            [_] => {
                self.unsupported(sp, &format!("class `{name}` with a base class that is not a natively compiled class"), "subclassing CPython classes is phase 4; keep this class in a module that falls back to CPython");
                None
            }
            _ => {
                self.unsupported(sp, &format!("multiple inheritance in `{name}`"), "use single inheritance");
                None
            }
        };
        let mut methods = Vec::new();
        let mut class_vars: Vec<String> = Vec::new();
        let mut fields: Vec<String> = Vec::new();
        for item in arr(st, "body") {
            let isp = self.span(item);
            match ty(item) {
                "FunctionDef" => {
                    let mname = sstr(item, "name").to_string();
                    let mut kind = MethodKind::Instance;
                    for d in arr(item, "decorator_list") {
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
        let unit_prefix = if self.unit == 0 { String::new() } else { format!("{}_", self.ld.units[self.unit].name) };
        const RESERVED: &[&str] = &["Box", "Vec", "String", "Option", "Result", "Some", "None", "Ok", "Err", "Rc", "RefCell", "Default", "Clone", "Copy", "Self", "Str"];
        let rust = if RESERVED.contains(&name.as_str()) { format!("{unit_prefix}Py{name}") } else { format!("{unit_prefix}{name}") };
        let qual = if self.unit == 0 { name.clone() } else { format!("{}.{name}", self.ld.units[self.unit].name) };
        self.ld.classes.push(ClassDef { id: cid, name: qual, rust, unit: self.unit, span: sp, base, methods, fields, class_vars, dataclass });
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
                    self.cur_fid = None;
                    self.locals = self.module_vars.difference(&self.globals).cloned().collect();
                    self.in_module = true;
                    let Some(value) = self.lower_expr(&item["value"]) else { continue };
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
        let sig = CallSig { names: params.clone(), n_posonly: 0, n_pos: params.len(), defaults: sig_defaults, vararg: None, kwarg: None };
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
    fn lower_genexp(&mut self, e: &Value) -> Option<Expr> {
        let sp = self.span(e);
        let loc = &e["_loc"];
        let gens = arr(e, "generators");
        if gens.iter().any(|g| g["is_async"].as_u64().unwrap_or(0) != 0) {
            self.unsupported(sp, "async generator expression", "use an async generator function");
            return None;
        }
        let first = self.lower_expr(&gens[0]["iter"])?;
        let mut body = serde_json::json!([{"_t": "Expr", "_loc": loc, "value": {"_t": "Yield", "_loc": loc, "value": e["elt"]}}]);
        for (gi, g) in gens.iter().enumerate().rev() {
            for c in arr(g, "ifs").iter().rev() {
                body = serde_json::json!([{"_t": "If", "_loc": loc, "test": c, "body": body, "orelse": []}]);
            }
            let iter = if gi == 0 { serde_json::json!({"_t": "Name", "id": "__genexpr_it", "_loc": loc}) } else { g["iter"].clone() };
            body = serde_json::json!([{"_t": "For", "_loc": loc, "target": g["target"], "iter": iter, "body": body, "orelse": []}]);
        }
        let def = serde_json::json!({
            "_t": "FunctionDef", "name": "<genexpr>", "decorator_list": [], "returns": null, "_loc": loc, "body": body,
            "args": {"_t": "arguments", "posonlyargs": [], "args": [{"_t": "arg", "arg": "__genexpr_it", "annotation": null, "_loc": loc}],
                     "vararg": null, "kwonlyargs": [], "kw_defaults": [], "kwarg": null, "defaults": []},
        });
        let fid = self.ld.alloc_func();
        let (params, sig) = signature(&def["args"]);
        self.ld.func_anns.insert(fid, (params.iter().map(|_| None).collect(), None));
        self.ld.func_sigs.insert(fid, call_sig(&sig, self.unit));
        let func = self.lower_nested(fid, &def)?;
        Some(Expr { kind: ExprKind::CallObj { func: Box::new(func), args: vec![first], kwargs: vec![] }, span: sp })
    }

    fn lower_comp(&mut self, e: &Value) -> Option<Expr> {
        let sp = self.span(e);
        if ty(e) == "GeneratorExp" {
            return self.lower_genexp(e);
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
                let it = self.lower_expr(&g["iter"]);
                self.comp_scopes.extend(saved);
                self.comp_owner.extend(saved_owner);
                it?
            } else {
                self.lower_expr(&g["iter"])?
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
        let sp = self.span(e);
        let f = &e["func"];
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
                        return Some(Expr { kind: ExprKind::New { class: c, args, kwargs }, span: sp });
                    }
                    Binding::Unit(u) => match self.ld.unit_funcs[u].get(attr).copied() {
                        Some(fid) => {
                            let (args, order) = self.bind_args(fid, &format!("{modname}.{attr}"), args, kwargs, sp)?;
                            let args = self.annotate_args(fid, args);
                            return Some(Expr { kind: ExprKind::Call { callee: Callee::Func(fid), args, order }, span: sp });
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
