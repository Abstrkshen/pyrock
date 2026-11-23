//! Test suites from design section 12:
//! - differential: CPython vs generated binary (stdout, exit status, exception line)
//! - golden: snapshot of generated Rust and the explain report
//! - diagnostics: snapshot of `pyrock check` output for rejected programs
//!   (a first line `# mode: native-only` selects that mode)
//! - interop: programs importing Rust/C modules (CPython cannot run them),
//!   checked against `expected.stdout` / `expected.stderr` (last line)
//!
//! Set `UPDATE_SNAPSHOTS=1` to (re)write golden and diagnostics snapshots.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn pyrock() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pyrock"));
    c.env("PYROCK_TARGET_DIR", root().join("target/pyrock-tests/target"));
    c
}

fn python() -> String {
    std::env::var("PYROCK_TEST_PYTHON").unwrap_or_else(|_| "python3".into())
}

fn cases(dir: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root().join("tests").join(dir))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "py"))
        // `_name.py` files are helper modules imported by other cases.
        .filter(|p| !p.file_name().unwrap().to_str().unwrap().starts_with('_'))
        .collect();
    v.sort();
    v
}

fn last_line(s: &[u8]) -> String {
    String::from_utf8_lossy(s).lines().last().unwrap_or("").to_string()
}

fn snapshot(path: PathBuf, actual: &str, failures: &mut Vec<String>) {
    if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    match std::fs::read_to_string(&path) {
        Ok(expected) if expected == actual => {}
        Ok(expected) => failures.push(format!("{}: snapshot differs\n--- expected\n{expected}\n--- actual\n{actual}", path.display())),
        Err(_) => failures.push(format!("{}: missing snapshot (run with UPDATE_SNAPSHOTS=1)", path.display())),
    }
}

#[test]
fn differential() {
    let mut failures = Vec::new();
    for case in cases("differential") {
        let name = case.file_stem().unwrap().to_str().unwrap().to_string();
        let out_dir = root().join("target/pyrock-tests").join(&name);
        let build = pyrock().arg("build").arg(&case).arg("--out").arg(&out_dir).output().unwrap();
        if !build.status.success() {
            failures.push(format!("{name}: pyrock build failed\n{}", String::from_utf8_lossy(&build.stderr)));
            continue;
        }
        let exe = String::from_utf8_lossy(&build.stdout).trim().to_string();
        let native = Command::new(&exe).output().unwrap();
        let cpython = Command::new(python()).arg(&case).output().unwrap();
        if native.stdout != cpython.stdout {
            failures.push(format!(
                "{name}: stdout differs\n--- cpython\n{}--- native\n{}",
                String::from_utf8_lossy(&cpython.stdout),
                String::from_utf8_lossy(&native.stdout)
            ));
        }
        if native.status.code() != cpython.status.code() {
            failures.push(format!("{name}: exit status cpython={:?} native={:?}", cpython.status.code(), native.status.code()));
        }
        let (ne, ce) = (last_line(&native.stderr), last_line(&cpython.stderr));
        if ne != ce {
            failures.push(format!("{name}: exception differs\n  cpython: {ce}\n  native:  {ne}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

#[test]
fn golden() {
    let mut failures = Vec::new();
    for case in cases("golden") {
        let name = case.file_stem().unwrap().to_str().unwrap().to_string();
        let out_dir = root().join("target/pyrock-tests/golden").join(&name);
        let emit = pyrock().arg("emit").arg(&case).arg("--out").arg(&out_dir).output().unwrap();
        assert!(emit.status.success(), "{name}: emit failed: {}", String::from_utf8_lossy(&emit.stderr));
        let rs = std::fs::read_to_string(out_dir.join("src/main.rs")).unwrap();
        snapshot(case.with_extension("rs.golden"), &rs, &mut failures);
        let explain = pyrock().current_dir(root()).arg("explain").arg(case.strip_prefix(root()).unwrap()).output().unwrap();
        snapshot(case.with_extension("explain.golden"), &String::from_utf8_lossy(&explain.stdout), &mut failures);
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

#[test]
fn diagnostics() {
    let mut failures = Vec::new();
    for case in cases("diagnostics") {
        let rel = case.strip_prefix(root()).unwrap();
        let src = std::fs::read_to_string(&case).unwrap();
        let mut cmd = pyrock();
        cmd.current_dir(root()).arg("check").arg(rel);
        if src.starts_with("# mode: native-only") {
            cmd.args(["--mode", "native-only"]);
        }
        let out = cmd.output().unwrap();
        if out.status.success() {
            failures.push(format!("{}: expected `check` to fail", rel.display()));
        }
        snapshot(case.with_extension("stderr"), &String::from_utf8_lossy(&out.stderr), &mut failures);
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

#[test]
fn interop() {
    let mut failures = Vec::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root().join("tests/interop")).unwrap().map(|e| e.unwrap().path()).filter(|p| p.join("main.py").exists()).collect();
    dirs.sort();
    for dir in dirs {
        let name = dir.file_name().unwrap().to_str().unwrap().to_string();
        let out_dir = root().join("target/pyrock-tests/interop").join(&name);
        let build = pyrock().arg("build").arg(dir.join("main.py")).arg("--out").arg(&out_dir).output().unwrap();
        if !build.status.success() {
            failures.push(format!("{name}: pyrock build failed\n{}", String::from_utf8_lossy(&build.stderr)));
            continue;
        }
        let exe = String::from_utf8_lossy(&build.stdout).trim().to_string();
        let run = Command::new(&exe).output().unwrap();
        let want = std::fs::read_to_string(dir.join("expected.stdout")).unwrap_or_default();
        if String::from_utf8_lossy(&run.stdout) != want {
            failures.push(format!("{name}: stdout differs\n--- expected\n{want}--- native\n{}", String::from_utf8_lossy(&run.stdout)));
        }
        let want_err = std::fs::read_to_string(dir.join("expected.stderr")).map(|s| s.trim().to_string()).unwrap_or_default();
        let got_err = last_line(&run.stderr);
        if got_err != want_err {
            failures.push(format!("{name}: last stderr line\n  expected: {want_err}\n  native:   {got_err}"));
        }
        let want_code = if want_err.is_empty() { 0 } else { 1 };
        if run.status.code() != Some(want_code) {
            failures.push(format!("{name}: exit status {:?}, expected {want_code}", run.status.code()));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// `--python` given as a relative path must still reach PyO3's build script,
/// which cargo runs from another directory (regression: the README form
/// `--python ../.venv/bin/python` failed at `run`).
#[test]
fn relative_python_path() {
    let py = Command::new(python()).args(["-c", "import sys; print(sys.executable)"]).output().unwrap();
    let abs = String::from_utf8_lossy(&py.stdout).trim().to_string();
    let rel = abs.trim_start_matches('/');
    let case = root().join("tests/cli/fallback_imports/main.py");
    let out_dir = root().join("target/pyrock-tests/relative-python");
    let build = pyrock().current_dir("/").arg("build").arg(&case).arg("--python").arg(rel).arg("--out").arg(&out_dir).output().unwrap();
    assert!(build.status.success(), "build with --python {rel} from / failed:\n{}", String::from_utf8_lossy(&build.stderr));
    // Generated code must compile without warnings: `pyrock run` shows them above the program's output.
    let err = String::from_utf8_lossy(&build.stderr);
    assert!(!err.contains("warning"), "cargo warnings in generated code:\n{err}");
    let exe = String::from_utf8_lossy(&build.stdout).trim().to_string();
    let run = Command::new(&exe).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
}
