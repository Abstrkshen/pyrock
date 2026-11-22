//! `pyfast` command-line driver: parse → infer → plan → emit → cargo.

use pyfast_analysis::{plan, Plan, Tier};
use pyfast_infer::{generic, specialize, Program};
use pyfast_ir::{Diagnostic, Mode, Severity};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

const USAGE: &str = "\
pyfast — ahead-of-time Python → Rust compiler (prototype)

USAGE:
  pyfast check   <file.py>
  pyfast explain <file.py> [--format text|json]
  pyfast emit    <file.py> [--out DIR]
  pyfast build   <file.py> [--out DIR] [--release] [--mode hybrid|native-only]
  pyfast run     <file.py> [--release] [--mode ...] [-- ARGS...]
  pyfast bench   <file.py> [--baseline python] [--runs N]

OPTIONS:
  --python PATH   interpreter used for parsing and baselines (default: python3)
";

struct Opts {
    cmd: String,
    file: String,
    out: Option<PathBuf>,
    format: String,
    release: bool,
    mode: Mode,
    python: String,
    runs: usize,
    passthrough: Vec<String>,
}

fn parse_args() -> Result<Opts, String> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().ok_or(USAGE)?;
    if cmd == "-h" || cmd == "--help" {
        return Err(USAGE.into());
    }
    if cmd == "-V" || cmd == "--version" || cmd == "version" {
        println!("pyfast {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }
    let mut o = Opts {
        cmd,
        file: String::new(),
        out: None,
        format: "text".into(),
        release: false,
        mode: Mode::Hybrid,
        python: "python3".into(),
        runs: 5,
        passthrough: vec![],
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => o.out = Some(args.next().ok_or("--out needs a value")?.into()),
            "--format" => o.format = args.next().ok_or("--format needs a value")?,
            "--release" => o.release = true,
            "--python" => o.python = args.next().ok_or("--python needs a value")?,
            "--runs" => o.runs = args.next().ok_or("--runs needs a value")?.parse().map_err(|_| "bad --runs")?,
            "--baseline" => {
                let b = args.next().ok_or("--baseline needs a value")?;
                if b != "python" {
                    return Err(format!("unknown baseline `{b}` (only `python` is supported)"));
                }
            }
            "--mode" => {
                o.mode = match args.next().as_deref() {
                    Some("hybrid") => Mode::Hybrid,
                    Some("native-only") => Mode::NativeOnly,
                    other => return Err(format!("unknown mode {other:?}")),
                }
            }
            "--" => o.passthrough.extend(args.by_ref()),
            s if s.starts_with('-') => return Err(format!("unknown option `{s}`\n\n{USAGE}")),
            s => {
                if !o.file.is_empty() {
                    return Err(format!("unexpected argument `{s}`"));
                }
                o.file = s.to_string();
            }
        }
    }
    if o.file.is_empty() {
        return Err(format!("missing input file\n\n{USAGE}"));
    }
    // A relative interpreter path (`../.venv/bin/python`) must survive cargo running
    // PyO3's build script from another directory. `absolute`, not `canonicalize`:
    // resolving the venv symlink would lose the venv's site-packages.
    if o.python.contains('/') {
        if let Ok(p) = std::path::absolute(&o.python) {
            o.python = p.display().to_string();
        }
    }
    Ok(o)
}

fn main() -> ExitCode {
    let o = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    match run(&o) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("pyfast: {e}");
            ExitCode::from(1)
        }
    }
}

struct Compiled<'m> {
    prog: Program<'m>,
    plan: Plan,
    diagnostics: Vec<Diagnostic>,
}

fn has_errors(d: &[Diagnostic]) -> bool {
    d.iter().any(|d| d.severity == Severity::Error)
}

fn print_diags(files: &[String], d: &[Diagnostic]) {
    for x in d {
        eprintln!("{}", x.render_in(files));
    }
}

fn mode_str(m: Mode) -> &'static str {
    match m {
        Mode::Hybrid => "hybrid",
        Mode::NativeOnly => "native-only",
    }
}

fn run(o: &Opts) -> Result<ExitCode, String> {
    let file_name = Path::new(&o.file).file_name().and_then(|s| s.to_str()).unwrap_or("main.py").to_string();
    let parsed = pyfast_frontend::parse_file(&o.file, &o.python, o.mode)?;
    let Some(module) = parsed.module else {
        print_diags(&[o.file.clone()], &parsed.diagnostics);
        return Ok(ExitCode::from(1));
    };
    let files = module.files();
    if o.cmd == "bench" {
        return bench(o, &file_name);
    }
    let mut diagnostics = parsed.diagnostics;
    // Inference needs a well-formed HIR; stop early on frontend errors.
    let compiled = if has_errors(&diagnostics) {
        None
    } else {
        let prog = specialize(&module, o.mode);
        diagnostics.extend(prog.diagnostics.iter().cloned());
        let (plan, plan_diags) = plan(&prog);
        diagnostics.extend(plan_diags);
        Some(Compiled { prog, plan, diagnostics: diagnostics.clone() })
    };

    match o.cmd.as_str() {
        "check" => {
            print_diags(&files, &diagnostics);
            if has_errors(&diagnostics) {
                return Ok(ExitCode::from(1));
            }
            let c = compiled.unwrap();
            let (n, b) = c.plan.counts();
            println!(
                "{}: ok ({}) — {n} native and {b} bridged specialization(s), {} CPython boundary site(s), {} linked Rust/C module(s)",
                o.file,
                mode_str(o.mode),
                c.plan.boundaries.len(),
                c.plan.externs_used.len()
            );
            Ok(ExitCode::SUCCESS)
        }
        "explain" => {
            let schemes = generic::infer_schemes(&module);
            if o.format == "json" {
                println!("{}", serde_json::to_string_pretty(&report_json(o, &parsed.python_version, &module, &schemes, compiled.as_ref(), &diagnostics)).unwrap());
            } else {
                explain_text(o, &parsed.python_version, &module, &schemes, compiled.as_ref(), &diagnostics);
            }
            Ok(if has_errors(&diagnostics) { ExitCode::from(1) } else { ExitCode::SUCCESS })
        }
        "emit" | "build" | "run" => {
            if has_errors(&diagnostics) {
                print_diags(&files, &diagnostics);
                return Ok(ExitCode::from(1));
            }
            let c = compiled.unwrap();
            let out_dir = o.out.clone().unwrap_or_else(|| default_out(&o.file));
            write_project(o, &parsed.python_version, &module, &c, &out_dir, &file_name)?;
            if o.cmd == "emit" {
                println!("wrote {}", out_dir.display());
                return Ok(ExitCode::SUCCESS);
            }
            let exe = cargo_build(&out_dir, o.release, true, &o.python)?;
            if o.cmd == "build" {
                println!("{}", exe.display());
                return Ok(ExitCode::SUCCESS);
            }
            let status = Command::new(&exe).args(&o.passthrough).status().map_err(|e| e.to_string())?;
            Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

fn default_out(file: &str) -> PathBuf {
    let p = Path::new(file);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("main");
    p.parent().unwrap_or(Path::new(".")).join(".pyfast").join(stem)
}

/// Cargo package name: the script stem plus a hash of the absolute output
/// directory, so projects never collide in the shared build cache (e.g.
/// many different `main.py` programs).
fn crate_name(file_name: &str, out: &Path) -> String {
    let stem = file_name.trim_end_matches(".py");
    let s: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    let abs = std::path::absolute(out).unwrap_or_else(|_| out.to_path_buf());
    let mut h: u32 = 2166136261; // FNV-1a: stable across Rust versions
    for b in abs.to_string_lossy().bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    format!("pyfast_{s}_{h:08x}")
}

/// Shared Cargo target dir: `$PYFAST_TARGET_DIR`, else `$XDG_CACHE_HOME/pyfast/target`,
/// else `~/.cache/pyfast/target`. Keeps the runtime and PyO3 compiled once.
fn target_dir() -> PathBuf {
    if let Ok(p) = std::env::var("PYFAST_TARGET_DIR") {
        return PathBuf::from(p);
    }
    let cache = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".cache"));
    cache.join("pyfast").join("target")
}

fn runtime_path() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("PYFAST_RUNTIME") {
        return Ok(PathBuf::from(p));
    }
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime");
    p.canonicalize().map_err(|e| format!("cannot locate pyfast-runtime at {}: {e} (set PYFAST_RUNTIME)", p.display()))
}

/// The build interpreter's module search path, embedded into the binary so
/// runtime imports see the same packages (venvs included).
fn python_paths(python: &str, root_dir: &str) -> Vec<String> {
    let out = Command::new(python).args(["-c", "import sys, json; print(json.dumps(sys.path))"]).output();
    let mut paths = vec![root_dir.to_string()];
    if let Ok(o) = out {
        if let Ok(v) = serde_json::from_slice::<Vec<String>>(&o.stdout) {
            paths.extend(v.into_iter().filter(|p| !p.is_empty()));
        }
    }
    paths.dedup();
    paths
}

fn write_project(o: &Opts, py: &str, module: &pyfast_ir::Module, c: &Compiled, out: &Path, file_name: &str) -> Result<(), String> {
    let py_paths = if c.plan.uses_python { python_paths(&o.python, &module.root_dir) } else { vec![] };
    let emitted = pyfast_codegen_rust::emit(&c.prog, &c.plan, file_name, &py_paths);
    std::fs::create_dir_all(out.join("src")).map_err(|e| e.to_string())?;
    let rt = runtime_path()?;
    let features = if c.plan.uses_python { ", features = [\"python\"]" } else { "" };
    let build_deps = if emitted.build_rs.is_some() { "\n[build-dependencies]\ncc = \"1\"\n" } else { "" };
    let cargo = format!(
        "# Generated by pyfast. Do not edit.\n[package]\nname = \"{}\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n[dependencies]\npyfast-runtime = {{ path = {:?}{features} }}\n{build_deps}\n[profile.release]\ncodegen-units = 1\npanic = \"abort\"\n\n[workspace]\n",
        crate_name(file_name, out),
        rt.display().to_string()
    );
    write_if_changed(&out.join("Cargo.toml"), &cargo)?;
    write_if_changed(&out.join("src/main.rs"), &emitted.main_rs)?;
    match &emitted.build_rs {
        Some(b) => write_if_changed(&out.join("build.rs"), b)?,
        None => {
            let _ = std::fs::remove_file(out.join("build.rs"));
        }
    }
    let files = module.files();
    let smap: Vec<_> = emitted
        .source_map
        .iter()
        .map(|(f, s)| serde_json::json!({"rust_fn": f, "python": {"file": files.get(s.unit as usize), "line": s.line, "col": s.col}}))
        .collect();
    write_if_changed(&out.join("pyfast.sourcemap.json"), &serde_json::to_string_pretty(&serde_json::json!({"file": file_name, "functions": smap})).unwrap())?;
    let schemes = generic::infer_schemes(module);
    let report = report_json(o, py, module, &schemes, Some(c), &c.diagnostics);
    write_if_changed(&out.join("pyfast-report.json"), &serde_json::to_string_pretty(&report).unwrap())?;
    Ok(())
}

/// Avoid touching mtimes so cargo's incremental cache stays warm.
fn write_if_changed(p: &Path, content: &str) -> Result<(), String> {
    if std::fs::read_to_string(p).ok().as_deref() == Some(content) {
        return Ok(());
    }
    std::fs::write(p, content).map_err(|e| format!("{}: {e}", p.display()))
}

fn cargo_bin() -> String {
    if let Ok(c) = std::env::var("CARGO") {
        return c;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let p = Path::new(&home).join(".cargo/bin/cargo");
    if p.exists() {
        p.display().to_string()
    } else {
        "cargo".into()
    }
}

/// Build the generated project; returns the executable path.
fn cargo_build(out: &Path, release: bool, quiet: bool, python: &str) -> Result<PathBuf, String> {
    let target = target_dir();
    let mut cmd = Command::new(cargo_bin());
    // PyO3 links against the same interpreter that parsed the program.
    cmd.arg("build").arg("--manifest-path").arg(out.join("Cargo.toml")).env("CARGO_TARGET_DIR", &target).env("PYO3_PYTHON", python);
    if release {
        cmd.arg("--release");
    }
    if quiet {
        cmd.arg("-q");
    }
    let st = cmd.status().map_err(|e| format!("failed to run cargo: {e}"))?;
    if !st.success() {
        return Err("cargo build of the generated project failed (if the error is inside an imported .rs/.c file, fix it there; otherwise this is a pyfast bug)".into());
    }
    let cargo_toml = std::fs::read_to_string(out.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let name = cargo_toml.lines().find_map(|l| l.strip_prefix("name = \"")).and_then(|l| l.strip_suffix('"')).unwrap_or("main");
    let built = target.join(if release { "release" } else { "debug" }).join(name);
    // Stable location next to the generated project: .pyfast/<stem>/<stem>
    let stem = name.strip_prefix("pyfast_").and_then(|n| n.rsplit_once('_')).map(|(s, _)| s).unwrap_or(name);
    let exe = out.join(stem);
    std::fs::copy(&built, &exe).map_err(|e| format!("copying {} to {}: {e}", built.display(), exe.display()))?;
    Ok(exe)
}

fn tier_str(t: Tier) -> &'static str {
    match t {
        Tier::Native => "native",
        Tier::Bridge => "bridge",
    }
}

fn explain_text(o: &Opts, py: &str, module: &pyfast_ir::Module, schemes: &generic::Schemes, c: Option<&Compiled>, diags: &[Diagnostic]) {
    let files = module.files();
    println!("pyfast explain {}   (mode: {}, parsed with CPython {py})\n", o.file, mode_str(o.mode));
    if !module.imports.is_empty() {
        println!("Imports");
        let w = module.imports.iter().map(|r| r.name.len()).max().unwrap_or(4);
        for r in &module.imports {
            let path = r.path.as_deref().map(|p| format!("  {p}")).unwrap_or_default();
            let note = r.note.as_deref().filter(|_| r.kind != "python (CPython fallback)").map(|n| format!("  ({n})")).unwrap_or_default();
            println!("  {:<w$}  -> {}{path}{note}", r.name, r.kind);
        }
        for x in &module.externs {
            for f in &x.funcs {
                let ps: Vec<String> = f.params.iter().map(|p| p.rust()).collect();
                println!("      {}.{}({}) -> {}{}", x.name, f.name, ps.join(", "), f.ret.rust(), if f.fallible { "  [Result: errors raise RuntimeError]" } else { "" });
            }
            for (n, why) in &x.skipped {
                println!("      {}.{n}: skipped ({why})", x.name);
            }
        }
        println!();
    }
    println!("Inferred generic signatures");
    for (fid, s) in &schemes.by_func {
        println!("  {}", generic::render(&module.funcs[*fid].name, s));
    }
    if let Some(c) = c {
        println!("\nSpecializations (reachable from module entry)");
        let w = c.plan.entries.iter().map(|e| e.signature.len()).max().unwrap_or(10);
        for e in c.plan.entries.iter().skip(1) {
            let rec = if e.recursive { "  recursive" } else { "" };
            println!("  {:<w$}  {:<7} {:<24} effects: {}{rec}", e.signature, tier_str(e.tier), e.rust_name, e.effects.join(", "));
        }
        if !c.plan.unreachable.is_empty() {
            println!("\nNot reached from module entry (not compiled): {}", c.plan.unreachable.join(", "));
        }
        if !c.plan.boundaries.is_empty() {
            println!("\nCPython boundaries");
            for b in &c.plan.boundaries {
                let f = files.get(b.span.unit as usize).map(|s| s.as_str()).unwrap_or("?");
                println!("  {f}:{}  {}  {}  [{}]", b.span, b.code, b.what, b.function);
                println!("      {}", b.reason);
            }
        }
        let (n, b) = c.plan.counts();
        let body = if c.plan.entries[0].tier == Tier::Native { "native" } else { "bridged" };
        println!("\nSummary: {n} native, {b} bridged specialization(s); module body {body}; {} linked Rust/C module(s)", c.plan.externs_used.len());
        println!("Runtime: {}", if c.plan.uses_python { "embeds CPython (libpython) for the boundaries above" } else { "no CPython needed" });
        println!("Int semantics: i64 subset with checked arithmetic (OverflowError instead of promotion)");
    }
    if !diags.is_empty() {
        println!("\nDiagnostics");
        for d in diags {
            println!("  {}", d.render_in(&files).replace('\n', "\n  "));
        }
    }
}

fn report_json(o: &Opts, py: &str, module: &pyfast_ir::Module, schemes: &generic::Schemes, c: Option<&Compiled>, diags: &[Diagnostic]) -> serde_json::Value {
    let generics: Vec<_> = schemes.by_func.iter().map(|(fid, s)| generic::render(&module.funcs[*fid].name, s)).collect();
    let (specs, unreachable, boundaries) = match c {
        Some(c) => (
            serde_json::to_value(&c.plan.entries[1..]).unwrap(),
            serde_json::to_value(&c.plan.unreachable).unwrap(),
            serde_json::to_value(&c.plan.boundaries).unwrap(),
        ),
        None => (serde_json::json!([]), serde_json::json!([]), serde_json::json!([])),
    };
    let (native, bridge) = c.map(|c| c.plan.counts()).unwrap_or((0, 0));
    let externs: Vec<_> = module
        .externs
        .iter()
        .map(|x| {
            serde_json::json!({
                "name": x.name, "lang": x.lang, "path": x.path, "header": x.header,
                "functions": x.funcs.iter().map(|f| serde_json::json!({
                    "name": f.name,
                    "params": f.params.iter().map(|p| p.rust()).collect::<Vec<_>>(),
                    "ret": f.ret.rust(),
                    "fallible": f.fallible,
                })).collect::<Vec<_>>(),
                "skipped": x.skipped,
            })
        })
        .collect();
    serde_json::json!({
        "pyfast_version": env!("CARGO_PKG_VERSION"),
        "file": o.file,
        "files": module.files(),
        "mode": o.mode,
        "parser": {"protocol": pyfast_frontend::PROTOCOL, "cpython": py},
        "int_semantics": "i64-checked",
        "imports": module.imports,
        "externs": externs,
        "generic_signatures": generics,
        "specializations": specs,
        "unreachable_functions": unreachable,
        "boundaries": boundaries,
        "counts": {"native": native, "bridge": bridge},
        "diagnostics": diags,
        "runtime_requirements": {
            "cpython": c.map(|c| c.plan.uses_python).unwrap_or(false),
            "c_compiler": module.externs.iter().any(|x| x.lang == pyfast_ir::Lang::C),
        },
    })
}

fn bench(o: &Opts, file_name: &str) -> Result<ExitCode, String> {
    let exe_path = std::env::current_exe().map_err(|e| e.to_string())?;
    let out_dir = o.out.clone().unwrap_or_else(|| default_out(&o.file));
    // Reuse the normal build path in release mode.
    let st = Command::new(&exe_path)
        .args(["build", &o.file, "--release", "--python", &o.python, "--out"])
        .arg(&out_dir)
        .stdout(std::process::Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if !st.status.success() {
        eprint!("{}", String::from_utf8_lossy(&st.stderr));
        return Ok(ExitCode::from(1));
    }
    let exe = String::from_utf8_lossy(&st.stdout).trim().to_string();
    let time = |cmd: &mut Command| -> Result<(Duration, Vec<u8>), String> {
        let t = Instant::now();
        let out = cmd.output().map_err(|e| e.to_string())?;
        Ok((t.elapsed(), out.stdout))
    };
    let mut native = Vec::new();
    let mut python = Vec::new();
    let mut outputs_match = true;
    for _ in 0..o.runs {
        let (tn, on) = time(&mut Command::new(&exe))?;
        let (tp, op) = time(Command::new(&o.python).arg(&o.file))?;
        outputs_match &= on == op;
        native.push(tn);
        python.push(tp);
    }
    let stats = |v: &mut Vec<Duration>| {
        v.sort();
        (v[v.len() / 2], v[0], v[v.len() - 1])
    };
    let (nm, nmin, nmax) = stats(&mut native);
    let (pm, pmin, pmax) = stats(&mut python);
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("model name")).map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string()))
        .unwrap_or_else(|| "unknown".into());
    let cargo = cargo_bin();
    let rustc_path = Path::new(&cargo).parent().map(|d| d.join("rustc")).filter(|p| p.exists()).unwrap_or_else(|| "rustc".into());
    let rustc = Command::new(rustc_path).arg("--version").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let pyv = Command::new(&o.python).arg("--version").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    println!("pyfast bench {file_name}  ({} runs each, wall clock incl. process startup)", o.runs);
    println!("  hardware : {cpu}");
    println!("  versions : {pyv}; {rustc}; pyfast {}", env!("CARGO_PKG_VERSION"));
    println!("  command  : {} {} | {exe}", o.python, o.file);
    println!("  python   : median {:>9.3} ms   (min {:.3}, max {:.3})", ms(pm), ms(pmin), ms(pmax));
    println!("  native   : median {:>9.3} ms   (min {:.3}, max {:.3})", ms(nm), ms(nmin), ms(nmax));
    println!("  speedup  : {:.2}x (median python / median native)", pm.as_secs_f64() / nm.as_secs_f64());
    println!("  stdout   : {}", if outputs_match { "identical in every run" } else { "MISMATCH — results are not comparable" });
    Ok(if outputs_match { ExitCode::SUCCESS } else { ExitCode::from(1) })
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
