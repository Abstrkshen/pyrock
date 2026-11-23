//! The playground pipeline (AST JSON in, reports out) gives the same answers as the CLI.

use pyrock_ir::Mode;
use std::io::Write;
use std::process::Command;

fn ast_json(src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("pyrock-playground-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("main_{}.py", src.len()));
    std::fs::File::create(&path).unwrap().write_all(src.as_bytes()).unwrap();
    let out = Command::new("python3").args(["-I", "-c", pyrock_frontend::dump_ast_source()]).arg(&path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn specializes_and_emits_rust() {
    let src = "def add(a, b):\n    return a + b\n\nprint(add(1, 2), add(\"py\", \"rock\"))\n";
    let r = pyrock_playground::compile("main.py", &ast_json(src), Mode::NativeOnly);
    assert_eq!(r["ok"], true, "{r}");
    let rust = r["rust"].as_str().unwrap();
    assert!(rust.contains("fn add__int_int(") && rust.contains("fn add__str_str("), "{rust}");
    let explain = r["explain"].as_str().unwrap();
    assert!(explain.contains("add(A, B) -> R where Add(A, B, R)"), "{explain}");
    assert!(explain.contains("Runtime: no CPython needed"), "{explain}");
    assert!(r["check"].as_str().unwrap().contains("main.py: ok (native-only)"), "{r}");
}

#[test]
fn reports_boundaries_and_native_only_errors() {
    let src = "import json\nprint(json.dumps([1, 2]))\n";
    let hybrid = pyrock_playground::compile("main.py", &ast_json(src), Mode::Hybrid);
    assert_eq!(hybrid["ok"], true, "{hybrid}");
    assert_eq!(hybrid["uses_python"], true);
    assert!(hybrid["explain"].as_str().unwrap().contains("P200  runtime import of `json`"), "{hybrid}");
    let native = pyrock_playground::compile("main.py", &ast_json(src), Mode::NativeOnly);
    assert_eq!(native["ok"], false);
    assert!(native["rust"].is_null());
    assert!(native["check"].as_str().unwrap().contains("P200  error: runtime import of `json`"), "{native}");
}

#[test]
fn syntax_errors_come_back_as_diagnostics() {
    let r = pyrock_playground::compile("main.py", &ast_json("def f(:\n    pass\n"), Mode::Hybrid);
    assert_eq!(r["ok"], false);
    assert!(r["check"].as_str().unwrap().contains("P001"), "{r}");
}

#[test]
fn local_imports_are_not_read_from_disk() {
    // There is only one file in the playground: `import helpers` is a CPython import.
    let r = pyrock_playground::compile("main.py", &ast_json("import helpers\n"), Mode::Hybrid);
    assert!(r["explain"].as_str().unwrap().contains("helpers"), "{r}");
}
