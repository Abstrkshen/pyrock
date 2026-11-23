//! Embed the runtime crate's sources into the `pyrock` binary, so an
//! installed binary does not depend on this repository staying in place.
//! The driver extracts them into the cache on first use (`runtime_path`).

use std::fmt::Write;
use std::path::Path;

fn main() {
    let rt = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime");
    println!("cargo:rerun-if-changed={}", rt.join("Cargo.toml").display());
    println!("cargo:rerun-if-changed={}", rt.join("src").display());
    let mut files: Vec<String> = std::fs::read_dir(rt.join("src"))
        .expect("runtime sources")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".rs"))
        .collect();
    files.sort();
    let mut out = String::from("/// (path inside the runtime crate, contents)\npub const RUNTIME_FILES: &[(&str, &str)] = &[\n");
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    let manifest = rt.join("Cargo.toml");
    feed(&std::fs::read(&manifest).unwrap());
    writeln!(out, "    (\"Cargo.toml\", include_str!({:?})),", manifest.canonicalize().unwrap().display().to_string()).unwrap();
    for f in &files {
        let p = rt.join("src").join(f);
        println!("cargo:rerun-if-changed={}", p.display());
        feed(f.as_bytes());
        feed(&std::fs::read(&p).unwrap());
        writeln!(out, "    (\"src/{f}\", include_str!({:?})),", p.canonicalize().unwrap().display().to_string()).unwrap();
    }
    out.push_str("];\n");
    writeln!(out, "/// Content hash of the embedded runtime (names its cache directory).\npub const RUNTIME_HASH: &str = \"{hash:016x}\";").unwrap();
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("runtime_files.rs");
    std::fs::write(dest, out).unwrap();
}
