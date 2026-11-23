//! The text report of `pyrock explain`, shared by the CLI and the browser playground.

use crate::{Plan, Tier};
use pyrock_infer::generic;
use pyrock_ir::{Diagnostic, Mode, Module};
use std::fmt::Write;

pub fn mode_str(m: Mode) -> &'static str {
    match m {
        Mode::Hybrid => "hybrid",
        Mode::NativeOnly => "native-only",
    }
}

pub fn tier_str(t: Tier) -> &'static str {
    match t {
        Tier::Native => "native",
        Tier::Bridge => "bridge",
    }
}

/// `plan` is None when the frontend reported errors (nothing was specialized).
pub fn explain_text(file: &str, mode: Mode, py: &str, module: &Module, schemes: &generic::Schemes, plan: Option<&Plan>, diags: &[Diagnostic]) -> String {
    let mut o = String::new();
    let files = module.files();
    let _ = writeln!(o, "pyrock explain {file}   (mode: {}, parsed with CPython {py})\n", mode_str(mode));
    if !module.imports.is_empty() {
        let _ = writeln!(o, "Imports");
        let w = module.imports.iter().map(|r| r.name.len()).max().unwrap_or(4);
        for r in &module.imports {
            let path = r.path.as_deref().map(|p| format!("  {p}")).unwrap_or_default();
            let note = r.note.as_deref().filter(|_| r.kind != "python (CPython fallback)").map(|n| format!("  ({n})")).unwrap_or_default();
            let _ = writeln!(o, "  {:<w$}  -> {}{path}{note}", r.name, r.kind);
        }
        for x in &module.externs {
            for f in &x.funcs {
                let ps: Vec<String> = f.params.iter().map(|p| p.rust()).collect();
                let _ = writeln!(o, "      {}.{}({}) -> {}{}", x.name, f.name, ps.join(", "), f.ret.rust(), if f.fallible { "  [Result: errors raise RuntimeError]" } else { "" });
            }
            for (n, why) in &x.skipped {
                let _ = writeln!(o, "      {}.{n}: skipped ({why})", x.name);
            }
        }
        let _ = writeln!(o);
    }
    let _ = writeln!(o, "Inferred generic signatures");
    for (fid, s) in &schemes.by_func {
        let _ = writeln!(o, "  {}", generic::render(&module.funcs[*fid].name, s));
    }
    if let Some(plan) = plan {
        let _ = writeln!(o, "\nSpecializations (reachable from module entry)");
        let w = plan.entries.iter().map(|e| e.signature.len()).max().unwrap_or(10);
        for e in plan.entries.iter().skip(1) {
            let rec = if e.recursive { "  recursive" } else { "" };
            let _ = writeln!(o, "  {:<w$}  {:<7} {:<24} effects: {}{rec}", e.signature, tier_str(e.tier), e.rust_name, e.effects.join(", "));
        }
        if !plan.unreachable.is_empty() {
            let _ = writeln!(o, "\nNot reached from module entry (not compiled): {}", plan.unreachable.join(", "));
        }
        if !plan.boundaries.is_empty() {
            let _ = writeln!(o, "\nCPython boundaries");
            for b in &plan.boundaries {
                let f = files.get(b.span.unit as usize).map(|s| s.as_str()).unwrap_or("?");
                let _ = writeln!(o, "  {f}:{}  {}  {}  [{}]", b.span, b.code, b.what, b.function);
                let _ = writeln!(o, "      {}", b.reason);
            }
        }
        let (n, b) = plan.counts();
        let body = if plan.entries[0].tier == Tier::Native { "native" } else { "bridged" };
        let _ = writeln!(o, "\nSummary: {n} native, {b} bridged specialization(s); module body {body}; {} linked Rust/C module(s)", plan.externs_used.len());
        let _ = writeln!(o, "Runtime: {}", if plan.uses_python { "embeds CPython (libpython) for the boundaries above" } else { "no CPython needed" });
        let _ = writeln!(o, "Int semantics: i64 subset with checked arithmetic (OverflowError instead of promotion)");
    }
    if !diags.is_empty() {
        let _ = writeln!(o, "\nDiagnostics");
        for d in diags {
            let _ = writeln!(o, "  {}", d.render_in(&files).replace('\n', "\n  "));
        }
    }
    o
}
