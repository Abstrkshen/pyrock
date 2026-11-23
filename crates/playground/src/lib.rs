//! The pyrock compiler for the browser playground (`site/play.html`).
//!
//! Built for `wasm32-unknown-unknown`. The page runs `dump_ast.py` in Pyodide
//! (CPython compiled to WebAssembly) and passes its JSON here; this module
//! runs the rest of the pipeline (lowering, inference, planning, Rust
//! codegen) and returns what `pyrock check`, `pyrock explain` and
//! `pyrock emit` would print. Building and running the Rust needs Cargo,
//! which a browser doesn't have.
//!
//! The exports use a minimal ABI with no generated glue: the host copies its
//! input into memory from [`alloc`], calls a function, then reads the UTF-8
//! result from [`out_ptr`] / [`out_len`].

use pyrock_analysis::explain::{explain_text, mode_str};
use pyrock_analysis::plan;
use pyrock_infer::{generic, specialize};
use pyrock_ir::{Mode, Severity};
use serde_json::{json, Value};
use std::cell::RefCell;

/// Compile one program from its `dump_ast.py` JSON. `file_name` is the name
/// shown in diagnostics and in the generated code.
pub fn compile(file_name: &str, ast_json: &str, mode: Mode) -> Value {
    let parsed = match pyrock_frontend::parse_ast_json(file_name, ast_json, mode) {
        Ok(p) => p,
        Err(e) => return json!({"ok": false, "check": format!("pyrock: {e}\n"), "explain": "", "rust": null}),
    };
    let Some(module) = parsed.module else {
        let check: String = parsed.diagnostics.iter().map(|d| d.render_in(&[file_name.to_string()]) + "\n").collect();
        return json!({"ok": false, "check": check, "explain": "", "rust": null});
    };
    let files = module.files();
    let mut diags = parsed.diagnostics;
    let errors = |d: &[pyrock_ir::Diagnostic]| d.iter().any(|d| d.severity == Severity::Error);
    let compiled = if errors(&diags) {
        None
    } else {
        let prog = specialize(&module, mode);
        diags.extend(prog.diagnostics.iter().cloned());
        let (plan, plan_diags) = plan(&prog);
        diags.extend(plan_diags);
        Some((prog, plan))
    };
    let ok = !errors(&diags);

    let mut check: String = diags.iter().map(|d| d.render_in(&files) + "\n").collect();
    if let (true, Some((_, plan))) = (ok, &compiled) {
        let (n, b) = plan.counts();
        check.push_str(&format!(
            "{file_name}: ok ({}) — {n} native and {b} bridged specialization(s), {} CPython boundary site(s), {} linked Rust/C module(s)\n",
            mode_str(mode),
            plan.boundaries.len(),
            plan.externs_used.len()
        ));
    }
    let schemes = generic::infer_schemes(&module);
    let explain = explain_text(file_name, mode, &parsed.python_version, &module, &schemes, compiled.as_ref().map(|(_, p)| p), &diags);
    let rust = match (&compiled, ok) {
        (Some((prog, plan)), true) => Value::String(pyrock_codegen_rust::emit(prog, plan, file_name, &[]).main_rs),
        _ => Value::Null,
    };
    let uses_python = compiled.as_ref().is_some_and(|(_, p)| p.uses_python);
    json!({"ok": ok, "check": check, "explain": explain, "rust": rust, "uses_python": uses_python})
}

thread_local! {
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn set_out(s: &str) {
    OUT.with(|o| *o.borrow_mut() = s.as_bytes().to_vec());
}

/// Reserve `len` bytes for the host to write an input into.
#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Free a buffer from [`alloc`].
///
/// # Safety
/// `ptr` and `len` must come from one [`alloc`] call, freed once.
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

/// Compile the AST JSON at `ptr..ptr+len`; `native_only` selects the mode.
/// The result (a JSON object) is in [`out_ptr`] / [`out_len`].
///
/// # Safety
/// `ptr..ptr+len` must be initialized memory from [`alloc`].
#[no_mangle]
pub unsafe extern "C" fn compile_ast(ptr: *const u8, len: usize, native_only: u32) {
    let input = std::slice::from_raw_parts(ptr, len);
    let text = String::from_utf8_lossy(input);
    let mode = if native_only != 0 { Mode::NativeOnly } else { Mode::Hybrid };
    set_out(&compile("main.py", &text, mode).to_string());
}

/// Put the parser script (`dump_ast.py`) in the output buffer.
#[no_mangle]
pub extern "C" fn dump_ast_source() {
    set_out(pyrock_frontend::dump_ast_source());
}

/// Put the compiler version in the output buffer.
#[no_mangle]
pub extern "C" fn version() {
    set_out(env!("CARGO_PKG_VERSION"));
}

#[no_mangle]
pub extern "C" fn out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}
