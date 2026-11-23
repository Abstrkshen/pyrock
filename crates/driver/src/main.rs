//! `pyrock` command-line driver: parse → infer → plan → emit → cargo.

use pyrock_analysis::explain::{self, mode_str};
use pyrock_analysis::{plan, Plan};
use pyrock_infer::{generic, specialize, Program};
use pyrock_ir::{Diagnostic, Mode, Severity};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

const USAGE: &str = "\
pyrock — ahead-of-time Python → Rust compiler

USAGE:
  pyrock check   <file.py> [--mode hybrid|native-only]
  pyrock explain <file.py> [--format text|json] [--mode ...]
  pyrock emit    <file.py> [--out DIR] [--mode ...]
  pyrock build   <file.py> [--out DIR] [--release] [--mode ...]
  pyrock run     <file.py> [--release] [--mode ...] [-- ARGS...]
  pyrock bench   <file.py> [--runs N] [--min-speedup X] [--hardware]

OPTIONS:
  --mode M          hybrid (default): code that can't be compiled runs in CPython;
                    native-only: report it as an error instead
  --release         build with optimizations (bench always does)
  --out DIR         where the generated Cargo project goes (default: .pyrock/<name>)
  --format F        explain output: text (default) or json
  --python PATH     interpreter used for parsing, embedding and the bench baseline
                    (default: python3)
  --runs N          bench: runs of each program (default: 5)
  --min-speedup X   bench: fail unless the native build is at least X times faster
  --hardware        bench: also print the CPU model
  -h, --help        print this help
  -V, --version     print the version

Docs: https://github.com/Abstrkshen/pyrock
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
    min_speedup: Option<f64>,
    hardware: bool,
    passthrough: Vec<String>,
}

fn parse_args() -> Result<Opts, String> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().ok_or(USAGE)?;
    if cmd == "-h" || cmd == "--help" || cmd == "help" {
        print!("{USAGE}");
        std::process::exit(0);
    }
    if cmd == "-V" || cmd == "--version" || cmd == "version" {
        println!("pyrock {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }
    if !matches!(cmd.as_str(), "check" | "explain" | "emit" | "build" | "run" | "bench") {
        return Err(format!("unknown command `{cmd}`\n\n{USAGE}"));
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
        min_speedup: None,
        hardware: false,
        passthrough: vec![],
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => o.out = Some(args.next().ok_or("--out needs a value")?.into()),
            "--format" => o.format = args.next().ok_or("--format needs a value")?,
            "--release" => o.release = true,
            "--python" => o.python = args.next().ok_or("--python needs a value")?,
            "--runs" => o.runs = args.next().ok_or("--runs needs a value")?.parse().ok().filter(|&n| n > 0).ok_or("--runs needs a positive number")?,
            "--min-speedup" => o.min_speedup = Some(args.next().ok_or("--min-speedup needs a value")?.parse().map_err(|_| "--min-speedup needs a number")?),
            "--hardware" => o.hardware = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
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
            eprintln!("pyrock: {e}");
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

fn run(o: &Opts) -> Result<ExitCode, String> {
    let file_name = Path::new(&o.file).file_name().and_then(|s| s.to_str()).unwrap_or("main.py").to_string();
    let parsed = pyrock_frontend::parse_file(&o.file, &o.python, o.mode)?;
    let Some(module) = parsed.module else {
        print_diags(&[o.file.clone()], &parsed.diagnostics);
        return Ok(ExitCode::from(1));
    };
    let files = explain::display_files(&module);
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
            // Replace this process with the program, so signals (`kill %1`,
            // a service manager's SIGTERM) and the exit status reach it
            // directly instead of leaving it running behind a dead `pyrock`.
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                let err = Command::new(&exe).args(&o.passthrough).exec();
                Err(format!("failed to run {}: {err}", exe.display()))
            }
            #[cfg(not(unix))]
            {
                let status = Command::new(&exe).args(&o.passthrough).status().map_err(|e| e.to_string())?;
                Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
            }
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

fn default_out(file: &str) -> PathBuf {
    let p = Path::new(file);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("main");
    p.parent().unwrap_or(Path::new(".")).join(".pyrock").join(stem)
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
    format!("pyrock_{s}_{h:08x}")
}

/// Shared Cargo target dir: `$PYROCK_TARGET_DIR`, else `$XDG_CACHE_HOME/pyrock/target`,
/// else `~/.cache/pyrock/target`. Keeps the runtime and PyO3 compiled once.
fn target_dir() -> PathBuf {
    if let Ok(p) = std::env::var("PYROCK_TARGET_DIR") {
        return PathBuf::from(p);
    }
    let cache = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".cache"));
    cache.join("pyrock").join("target")
}

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/runtime_files.rs"));
}

/// The runtime crate generated projects depend on: `$PYROCK_RUNTIME`, else
/// the copy embedded in this binary, extracted once per content hash into
/// `<cache>/pyrock/runtime/<hash>` (so the binary works from anywhere).
fn runtime_path() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("PYROCK_RUNTIME") {
        return Ok(PathBuf::from(p));
    }
    let base = target_dir().parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("runtime").join(embedded::RUNTIME_HASH);
    if dir.join("Cargo.toml").is_file() {
        return Ok(dir);
    }
    // Write to a fresh directory, then rename: concurrent builds never see a
    // half-written crate.
    let tmp = base.join("runtime").join(format!(".{}-{}", embedded::RUNTIME_HASH, std::process::id()));
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(tmp.join("src"))?;
        for (path, text) in embedded::RUNTIME_FILES {
            let text = if *path == "Cargo.toml" { standalone_manifest(text) } else { text.to_string() };
            std::fs::write(tmp.join(path), text)?;
        }
        Ok(())
    };
    write().map_err(|e| format!("cannot extract the pyrock runtime into {}: {e}", tmp.display()))?;
    if std::fs::rename(&tmp, &dir).is_err() {
        // Another process won the race.
        let _ = std::fs::remove_dir_all(&tmp);
    }
    Ok(dir)
}

/// The runtime's manifest without workspace inheritance.
fn standalone_manifest(text: &str) -> String {
    text.replace("version.workspace = true", &format!("version = {:?}", env!("CARGO_PKG_VERSION")))
        .replace("edition.workspace = true", "edition = \"2021\"")
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

fn write_project(o: &Opts, py: &str, module: &pyrock_ir::Module, c: &Compiled, out: &Path, file_name: &str) -> Result<(), String> {
    let py_paths = if c.plan.uses_python { python_paths(&o.python, &module.root_dir) } else { vec![] };
    let emitted = pyrock_codegen_rust::emit(&c.prog, &c.plan, file_name, &py_paths);
    std::fs::create_dir_all(out.join("src")).map_err(|e| e.to_string())?;
    let rt = runtime_path()?;
    let features = if c.plan.uses_python { ", features = [\"python\"]" } else { "" };
    // macOS framework Pythons (Xcode's /usr/bin/python3) are linked as
    // `@rpath/Python3.framework/...`: the binary needs an rpath to the
    // framework folder, which PyO3's build config knows.
    let framework_rpath = c.plan.uses_python && cfg!(target_os = "macos");
    let mut build_rs = emitted.build_rs.clone();
    if framework_rpath {
        let call = "    pyo3_build_config::add_python_framework_link_args();\n";
        build_rs = Some(match build_rs {
            Some(b) => b.replacen("fn main() {\n", &format!("fn main() {{\n{call}"), 1),
            None => format!("// Generated by pyrock: links the macOS Python framework.\nfn main() {{\n{call}}}\n"),
        });
    }
    let mut build_deps = String::new();
    if build_rs.is_some() {
        build_deps.push_str("\n[build-dependencies]\n");
        if emitted.build_rs.is_some() {
            build_deps.push_str("cc = \"1\"\n");
        }
        if framework_rpath {
            build_deps.push_str("pyo3-build-config = \"0.30\"\n");
        }
    }
    // Cargo dependencies declared by linked Rust modules (`// pyrock: dep regex = "1"`).
    let mut rust_deps = String::new();
    for (i, x) in module.externs.iter().enumerate() {
        if c.plan.externs_used.contains(&i) {
            for d in &x.deps {
                if !rust_deps.contains(d.as_str()) {
                    rust_deps.push_str(d);
                    rust_deps.push('\n');
                }
            }
        }
    }
    let cargo = format!(
        "# Generated by pyrock. Do not edit.\n[package]\nname = \"{}\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n[dependencies]\npyrock-runtime = {{ path = {:?}{features} }}\n{rust_deps}{build_deps}\n[profile.release]\ncodegen-units = 1\npanic = \"abort\"\n\n[workspace]\n",
        crate_name(file_name, out),
        rt.display().to_string()
    );
    write_if_changed(&out.join("Cargo.toml"), &cargo)?;
    write_if_changed(&out.join("src/main.rs"), &emitted.main_rs)?;
    match &build_rs {
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
    write_if_changed(&out.join("pyrock.sourcemap.json"), &serde_json::to_string_pretty(&serde_json::json!({"file": file_name, "functions": smap})).unwrap())?;
    let schemes = generic::infer_schemes(module);
    let report = report_json(o, py, module, &schemes, Some(c), &c.diagnostics);
    write_if_changed(&out.join("pyrock-report.json"), &serde_json::to_string_pretty(&report).unwrap())?;
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
    // Source paths compiled into the binary (panic locations in the runtime,
    // the standard library and dependencies) show the home directory as `~`,
    // so a shared binary doesn't carry the user name. Cargo appends this to
    // any `build.rustflags` already configured.
    if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        let flag = format!("--remap-path-prefix={}=~", home.to_string_lossy());
        cmd.arg("--config").arg(format!("build.rustflags=[{}]", serde_json::to_string(&flag).unwrap()));
    }
    if release {
        cmd.arg("--release");
    }
    if quiet {
        cmd.arg("-q");
    }
    let st = cmd.status().map_err(|e| format!("failed to run cargo: {e}"))?;
    if !st.success() {
        return Err("cargo build of the generated project failed (if the error is inside an imported .rs/.c file, fix it there; otherwise this is a pyrock bug)".into());
    }
    let cargo_toml = std::fs::read_to_string(out.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let name = cargo_toml.lines().find_map(|l| l.strip_prefix("name = \"")).and_then(|l| l.strip_suffix('"')).unwrap_or("main");
    let built = target.join(if release { "release" } else { "debug" }).join(name);
    // Stable location next to the generated project: .pyrock/<stem>/<stem>
    let stem = name.strip_prefix("pyrock_").and_then(|n| n.rsplit_once('_')).map(|(s, _)| s).unwrap_or(name);
    let exe = out.join(stem);
    // Copy next to it, then rename over it: replacing works even while the
    // previous build is still running ("Text file busy" when copying onto it).
    let tmp = exe.with_extension(format!("tmp{}", std::process::id()));
    std::fs::copy(&built, &tmp).map_err(|e| format!("copying {} to {}: {e}", built.display(), tmp.display()))?;
    std::fs::rename(&tmp, &exe).map_err(|e| format!("replacing {}: {e}", exe.display()))?;
    Ok(exe)
}

fn explain_text(o: &Opts, py: &str, module: &pyrock_ir::Module, schemes: &generic::Schemes, c: Option<&Compiled>, diags: &[Diagnostic]) {
    print!("{}", explain::explain_text(&o.file, o.mode, py, module, schemes, c.map(|c| &c.plan), diags));
}

fn report_json(o: &Opts, py: &str, module: &pyrock_ir::Module, schemes: &generic::Schemes, c: Option<&Compiled>, diags: &[Diagnostic]) -> serde_json::Value {
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
        "pyrock_version": env!("CARGO_PKG_VERSION"),
        "file": o.file,
        "files": module.files(),
        "mode": o.mode,
        "parser": {"protocol": pyrock_frontend::PROTOCOL, "cpython": py},
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
            "c_compiler": module.externs.iter().any(|x| x.lang == pyrock_ir::Lang::C),
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
    struct Run {
        time: Duration,
        out: std::process::Output,
    }
    let time = |cmd: &mut Command| -> Result<Run, String> {
        let t = Instant::now();
        let out = cmd.output().map_err(|e| e.to_string())?;
        Ok(Run { time: t.elapsed(), out })
    };
    // A failed run has no meaningful time: report it and stop.
    let failed = |who: &str, cmd: &str, r: &Run| {
        let err = String::from_utf8_lossy(&r.out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect();
        eprintln!("pyrock bench: the {who} run failed ({}), so there is nothing to compare: {cmd}", exit_desc(&r.out.status));
        for l in tail {
            eprintln!("  | {l}");
        }
    };
    let shown_exe = explain::display_path(&exe);
    let py_cmd = format!("{} {}", o.python, o.file);
    let mut native = Vec::new();
    let mut python = Vec::new();
    for _ in 0..o.runs {
        let n = time(&mut Command::new(&exe))?;
        let p = time(Command::new(&o.python).arg(&o.file))?;
        if !p.out.status.success() {
            failed("CPython", &py_cmd, &p);
            return Ok(ExitCode::from(1));
        }
        if !n.out.status.success() {
            failed("native", &shown_exe, &n);
            return Ok(ExitCode::from(1));
        }
        if n.out.stdout != p.out.stdout {
            eprintln!("pyrock bench: stdout differs between CPython and the native build, so the times are not comparable");
            eprintln!("  first difference at line {}", first_diff_line(&p.out.stdout, &n.out.stdout));
            return Ok(ExitCode::from(1));
        }
        native.push(n.time);
        python.push(p.time);
    }
    let stats = |v: &mut Vec<Duration>| {
        v.sort();
        (v[v.len() / 2], v[0], v[v.len() - 1])
    };
    let (nm, nmin, nmax) = stats(&mut native);
    let (pm, pmin, pmax) = stats(&mut python);
    let cargo = cargo_bin();
    let rustc_path = Path::new(&cargo).parent().map(|d| d.join("rustc")).filter(|p| p.exists()).unwrap_or_else(|| "rustc".into());
    let rustc = Command::new(rustc_path).arg("--version").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let pyv = Command::new(&o.python).arg("--version").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let speedup = pm.as_secs_f64() / nm.as_secs_f64();
    println!("pyrock bench {file_name}  ({} runs each, wall clock incl. process startup)", o.runs);
    if o.hardware {
        let cpu = std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|s| s.lines().find(|l| l.starts_with("model name")).map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string()))
            .or_else(|| Command::new("sysctl").args(["-n", "machdep.cpu.brand_string"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "unknown".into());
        println!("  hardware : {cpu}");
    }
    println!("  versions : {pyv}; {rustc}; pyrock {}", env!("CARGO_PKG_VERSION"));
    println!("  command  : {py_cmd} | {shown_exe}");
    println!("  python   : median {:>9.3} ms   (min {:.3}, max {:.3})", ms(pm), ms(pmin), ms(pmax));
    println!("  native   : median {:>9.3} ms   (min {:.3}, max {:.3})", ms(nm), ms(nmin), ms(nmax));
    println!("  speedup  : {speedup:.2}x (median python / median native)");
    println!("  output   : both exit 0, stdout identical in every run");
    if let Some(min) = o.min_speedup {
        if speedup < min {
            eprintln!("pyrock bench: speedup {speedup:.2}x is below --min-speedup {min}");
            return Ok(ExitCode::from(1));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn exit_desc(st: &std::process::ExitStatus) -> String {
    match st.code() {
        Some(c) => format!("exit status {c}"),
        None => "killed by a signal".into(),
    }
}

/// 1-based line number of the first difference between two outputs.
fn first_diff_line(a: &[u8], b: &[u8]) -> usize {
    let (a, b) = (String::from_utf8_lossy(a), String::from_utf8_lossy(b));
    let mut al = a.lines();
    let mut bl = b.lines();
    let mut n = 1;
    loop {
        match (al.next(), bl.next()) {
            (Some(x), Some(y)) if x == y => n += 1,
            (None, None) => return n,
            _ => return n,
        }
    }
}


fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
