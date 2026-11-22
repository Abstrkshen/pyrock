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
    };
    let (unit, diags) = ld.load_unit(&entry, "__main__", path);
    let Some(unit) = unit else {
        return Ok(ParseOutput { module: None, diagnostics: diags, python_version: ld.python_version });
    };
    let main = ld.units[unit].init;
    let funcs: Vec<Function> = ld.funcs.into_iter().map(|f| f.expect("function slot never filled")).collect();
    let module = Module {
        file: path.to_string(),
        funcs,
        main,
        units: ld.units,
        externs: ld.externs,
        slots: ld.slots,
        root_dir: root_dir.display().to_string(),
        imports: ld.imports,
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
#[derive(Clone, Copy, Debug)]
enum Binding {
    Func(FuncId),
    Unit(usize),
    Extern(usize),
    ExternFn(usize, usize),
    Slot(usize),
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
    locals: HashSet<String>,
    in_module: bool,
    /// Return annotation of the function being lowered.
    ret_ann: Option<Ty>,
    /// Import slots referenced by lowered code (unused `typing` imports are dropped).
    used_slots: HashSet<usize>,
    diags: Vec<Diagnostic>,
}

/// Modules whose imports only matter to type checkers.
const ANNOTATION_ONLY_MODULES: &[&str] = &["typing", "typing_extensions", "__future__"];

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

/// Collect names bound by assignment-like statements, in first-binding order.
/// Does not descend into nested function definitions.
fn collect_bindings(body: &[Value], out: &mut Vec<String>, top: bool) {
    let push = |n: &str, out: &mut Vec<String>| {
        if !out.iter().any(|x| x == n) {
            out.push(n.to_string());
        }
    };
    for st in body {
        match ty(st) {
            "Assign" => {
                for t in arr(st, "targets") {
                    if ty(t) == "Name" {
                        push(sstr(t, "id"), out);
                    }
                }
            }
            "AugAssign" | "AnnAssign" => {
                if ty(&st["target"]) == "Name" {
                    push(sstr(&st["target"], "id"), out);
                }
            }
            "For" => {
                if ty(&st["target"]) == "Name" {
                    push(sstr(&st["target"], "id"), out);
                }
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
                    if ty(&item["optional_vars"]) == "Name" {
                        push(sstr(&item["optional_vars"], "id"), out);
                    }
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
                let a = &st["args"];
                let params = arr(a, "args").iter().map(|p| parse_ann(&p["annotation"])).collect();
                self.ld.func_anns.insert(id, (params, parse_ann(&st["returns"])));
                self.func_ids.insert(name.clone(), (id, sp));
                self.bindings.insert(name.clone(), Binding::Func(id));
                self.ld.unit_funcs[self.unit].insert(name, id);
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
            if self.bindings.contains_key(name) {
                let sp = self.func_ids.get(name).map(|x| x.1).unwrap_or_default();
                self.err("P141", sp, format!("module-level name `{name}` rebinds a function or import; global rebinding is not compiled natively"));
            }
        }
        self.module_vars = module_locals.iter().cloned().collect();

        // Pass 2: lower function bodies, then the module body.
        let mut main_body = Vec::new();
        self.locals = self.module_vars.clone();
        self.in_module = true;
        for st in body {
            if ty(st) == "FunctionDef" {
                let name = sstr(st, "name");
                let Some(&(id, sp)) = self.func_ids.get(name) else { continue };
                if sp != self.span(st) {
                    continue; // duplicate, already reported
                }
                let saved = (std::mem::take(&mut self.locals), self.in_module);
                let f = self.lower_function(id, st);
                (self.locals, self.in_module) = saved;
                self.ld.funcs[id] = Some(f);
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
                        None => self.err("P140", sp, format!("`{module}.{attr}` is not a function of the natively compiled module `{module}`")),
                    },
                    Some(Binding::Extern(e)) if e != usize::MAX => match self.ld.externs[e].funcs.iter().position(|f| f.name == attr) {
                        Some(i) => {
                            self.bindings.insert(bind, Binding::ExternFn(e, i));
                        }
                        None => self.extern_missing(e, &attr, sp),
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
        let name = sstr(st, "name").to_string();
        let sp = self.span(st);
        let a = &st["args"];
        if !arr(st, "decorator_list").is_empty() {
            self.err("P101", sp, format!("decorators on `{name}` are not supported yet"));
        }
        if !arr(a, "posonlyargs").is_empty()
            || !arr(a, "kwonlyargs").is_empty()
            || !a["vararg"].is_null()
            || !a["kwarg"].is_null()
            || !arr(a, "defaults").is_empty()
        {
            self.err("P101", sp, format!("`{name}`: only fixed positional parameters are supported (no defaults, *args, **kwargs, / or *)"));
        }
        let params: Vec<String> = arr(a, "args").iter().map(|p| sstr(p, "arg").to_string()).collect();
        let mut locals = params.clone();
        collect_bindings(arr(st, "body"), &mut locals, false);
        self.locals = locals.iter().cloned().collect();
        self.in_module = false;
        let (param_ann, ret_ann) = self.ld.func_anns.get(&id).cloned().unwrap_or_default();
        self.ret_ann = ret_ann;
        let body = self.lower_block(arr(st, "body"));
        self.ret_ann = None;
        Function { id, name: self.qual(&name), params, locals, body, span: sp, is_module: false, unit: self.unit, param_ann, ret_ann }
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
                if targets.len() == 1 && ty(&targets[0]) != "Name" {
                    return self.lower_store(&targets[0], value, sp);
                }
                let mut names = Vec::new();
                for t in targets {
                    match self.name_target(t) {
                        Some(n) => names.push(n),
                        None => {
                            let tsp = self.span(t);
                            self.unsupported(tsp, &format!("assignment to `{}`", ty(t)), "tuple unpacking is not compiled yet");
                            return None;
                        }
                    }
                }
                if names.len() == 1 {
                    StmtKind::Assign { target: names.remove(0), value }
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
                    Some(target) => StmtKind::Assign { target, value },
                    None => return self.lower_store(&st["target"], value, sp),
                }
            }
            "AugAssign" => {
                let op = self.binop(&st["op"], sp)?;
                let value = self.lower_expr(&st["value"])?;
                let t = &st["target"];
                match self.name_target(t) {
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
                            self.unsupported(sp, "augmented assignment to this target", "use a temporary variable");
                            return None;
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
                self.unsupported(sp, "nested function definition", "closures are planned for milestone M6");
                return None;
            }
            "Import" | "ImportFrom" => {
                let actions = self.import_actions.remove(&sp).unwrap_or_default();
                let stmts: Vec<Stmt> = actions.into_iter().map(|a| Stmt { kind: StmtKind::Import(a), span: sp }).collect();
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
                    let name = h["name"].as_str().map(String::from);
                    handlers.push(Handler { types, name, body: self.lower_block(arr(h, "body")), span: hsp });
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
                        "Name" => Some(sstr(&item["optional_vars"], "id").to_string()),
                        _ => {
                            let vsp = self.span(&item["optional_vars"]);
                            self.unsupported(vsp, "unpacking in a `with` target", "bind one name and unpack it in the body");
                            return None;
                        }
                    };
                    body = vec![Stmt { kind: StmtKind::With { ctx, var, body }, span: sp }];
                }
                return body.into_iter().next();
            }
            "Global" | "Nonlocal" => {
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
            "Attribute" => StmtKind::SetAttr { obj: self.lower_expr(&t["value"])?, name: sstr(t, "attr").to_string(), value },
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
        let mut stmts = vec![Stmt { kind: StmtKind::Assign { target: first.clone(), value }, span: sp }];
        for n in &names[1..] {
            stmts.push(Stmt {
                kind: StmtKind::Assign { target: n.clone(), value: Expr { kind: ExprKind::Local(first.clone()), span: sp } },
                span: sp,
            });
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
        let Some(var) = self.name_target(&st["target"]) else {
            self.unsupported(sp, "tuple unpacking in `for`", "iterate and index explicitly for now");
            return None;
        };
        let it = &st["iter"];
        let is_range = ty(it) == "Call"
            && ty(&it["func"]) == "Name"
            && it["func"]["id"] == "range"
            && !self.locals.contains("range")
            && !self.bindings.contains_key("range")
            && arr(it, "keywords").is_empty()
            && (1..=3).contains(&arr(it, "args").len());
        if !is_range {
            let iter = self.lower_expr(it)?;
            let body = self.lower_block(arr(st, "body"));
            return Some(Stmt { kind: StmtKind::ForIter { var, iter, body }, span: sp });
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
        let body = self.lower_block(arr(st, "body"));
        Some(Stmt { kind: StmtKind::ForRange { var, start, stop, step, body }, span: sp })
    }

    fn binop(&mut self, op: &Value, sp: Span) -> Option<BinOp> {
        Some(match ty(op) {
            "Add" => BinOp::Add,
            "Sub" => BinOp::Sub,
            "Mult" => BinOp::Mul,
            "Div" => BinOp::Div,
            "FloorDiv" => BinOp::FloorDiv,
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
        if ty(idx) != "Slice" {
            return self.lower_expr(idx);
        }
        let sp = self.span(idx);
        let mut parts = Vec::new();
        for k in ["lower", "upper", "step"] {
            parts.push(if idx[k].is_null() { Expr { kind: ExprKind::NoneLit, span: sp } } else { self.lower_expr(&idx[k])? });
        }
        Some(self.call_obj(self.py_builtin("slice", sp), parts, sp))
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
            "Set" => {
                let items = Expr { kind: ExprKind::List(self.lower_exprs(arr(e, "elts"))?), span: sp };
                return Some(self.call_obj(self.py_builtin("set", sp), vec![items], sp));
            }
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
            "ListComp" | "DictComp" | "SetComp" | "GeneratorExp" | "Lambda" => {
                self.diags.push(
                    Diagnostic::error("P120", sp, format!("`{}` is not supported yet", ty(e)))
                        .note("comprehensions and lambdas need closures (milestone M6); use an explicit loop"),
                );
                return None;
            }
            other => {
                self.unsupported(sp, &format!("expression `{other}`"), "see docs/subset.md for the supported subset");
                return None;
            }
        };
        Some(Expr { kind, span: sp })
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
        Some(ExcType::Obj(self.lower_expr(t)?))
    }

    /// If `v` names a natively compiled module or extern, return its binding.
    fn module_binding(&self, v: &Value) -> Option<Binding> {
        if ty(v) != "Name" || self.locals.contains(sstr(v, "id")) {
            return None;
        }
        match self.bindings.get(sstr(v, "id")) {
            Some(b @ (Binding::Unit(_) | Binding::Extern(_))) => Some(*b),
            _ => None,
        }
    }

    fn lower_name(&mut self, id: &str, sp: Span) -> Option<Expr> {
        let kind = if self.locals.contains(id) {
            ExprKind::Local(id.to_string())
        } else if id == "__name__" {
            ExprKind::Str(if self.unit == 0 { "__main__".into() } else { self.ld.units[self.unit].name.clone() })
        } else if id == "__file__" {
            // CPython: the path as given for the entry script, absolute for imports.
            ExprKind::Str(self.ld.units[self.unit].path.clone())
        } else if let Some(b) = self.bindings.get(id).copied() {
            match b {
                Binding::Slot(s) => {
                    self.used_slots.insert(s);
                    ExprKind::Slot(s)
                }
                _ => {
                    self.unsupported(sp, &format!("using `{id}` as a first-class value"), "call it directly; function values are milestone M6");
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
        if arr(e, "args").iter().any(|a| ty(a) == "Starred") || arr(e, "keywords").iter().any(|k| k["arg"].is_null()) {
            self.unsupported(sp, "`*args` / `**kwargs` call", "pass arguments explicitly");
            return None;
        }
        let args = self.lower_exprs(arr(e, "args"))?;
        let mut kwargs = Vec::new();
        for k in arr(e, "keywords") {
            kwargs.push((sstr(k, "arg").to_string(), self.lower_expr(&k["value"])?));
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
            match self.bindings.get(id).copied() {
                Some(Binding::Func(fid)) => {
                    native_only(self, &format!("`{id}`"))?;
                    if self.in_module {
                        if let Some(&(_, def_sp)) = self.func_ids.get(id) {
                            if def_sp.line > sp.line {
                                self.err("P144", sp, format!("`{id}` is called before its definition (NameError at runtime)"));
                                return None;
                            }
                        }
                    }
                    let args = self.annotate_args(fid, args);
                    return Some(Expr { kind: ExprKind::Call { callee: Callee::Func(fid), args }, span: sp });
                }
                Some(Binding::ExternFn(ext, func)) => {
                    native_only(self, &format!("`{id}`"))?;
                    return Some(Expr { kind: ExprKind::ExternCall { ext, func, args }, span: sp });
                }
                Some(Binding::Unit(_) | Binding::Extern(_)) => {
                    self.unsupported(sp, &format!("calling module `{id}`"), "modules are not callable");
                    return None;
                }
                Some(Binding::Slot(_)) | None => {}
            }
            if self.builtin_exc(id) && kwargs.is_empty() && args.len() <= 1 {
                return Some(Expr { kind: ExprKind::NewExc { kind: id.to_string(), args }, span: sp });
            }
            if !self.bindings.contains_key(id) && !self.module_vars.contains(id) {
                if let Some(b) = Builtin::from_name(id) {
                    if kwargs.is_empty() {
                        return Some(Expr { kind: ExprKind::Call { callee: Callee::Builtin(b), args }, span: sp });
                    }
                    // e.g. print(x, end=""): use the real CPython builtin.
                }
            }
        }

        // Module attribute calls: helpers.f(...), mymath.dot(...).
        if ty(f) == "Attribute" {
            if let Some(b) = self.module_binding(&f["value"]) {
                let attr = sstr(f, "attr");
                let modname = sstr(&f["value"], "id").to_string();
                match b {
                    Binding::Unit(u) => match self.ld.unit_funcs[u].get(attr).copied() {
                        Some(fid) => {
                            native_only(self, &format!("`{modname}.{attr}`"))?;
                            let args = self.annotate_args(fid, args);
                            return Some(Expr { kind: ExprKind::Call { callee: Callee::Func(fid), args }, span: sp });
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

    /// Arguments to annotated parameters are checked/unboxed at the call.
    fn annotate_args(&self, fid: FuncId, args: Vec<Expr>) -> Vec<Expr> {
        let anns = self.ld.func_anns.get(&fid).map(|a| a.0.clone()).unwrap_or_default();
        args.into_iter().enumerate().map(|(i, a)| annotated(anns.get(i).copied().flatten(), a)).collect()
    }

    /// `f"a{x}b"` lowers to `"a" + str(x) + "b"`; format specs and `!r`
    /// conversions use CPython's `format`/`repr`.
    fn lower_fstring(&mut self, e: &Value) -> Option<Expr> {
        let sp = self.span(e);
        let str_call = |v: Expr, sp: Span| Expr { kind: ExprKind::Call { callee: Callee::Builtin(Builtin::Str), args: vec![v] }, span: sp };
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
