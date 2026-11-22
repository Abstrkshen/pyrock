//! Output and process entry. stdout is block-buffered (like CPython when
//! not attached to a terminal) and flushed before exit or a traceback.

use crate::exc::PyResult;
use crate::Str;
use std::io::{BufWriter, Write};

/// The stdout buffer, shared by every thread running compiled code.
static OUT: crate::Shared<Option<BufWriter<std::io::Stdout>>> = crate::Shared::new(None);

#[inline]
fn with_out<R>(f: impl FnOnce(&mut BufWriter<std::io::Stdout>) -> R) -> R {
    OUT.with(|o| f(o.get_or_insert_with(|| BufWriter::with_capacity(1 << 16, std::io::stdout()))))
}

/// `print(*parts)` with the default `sep=" "` and `end="\n"`.
pub fn print(parts: &[Str]) {
    with_out(|o| {
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                let _ = o.write_all(b" ");
            }
            let _ = o.write_all(p.as_bytes());
        }
        let _ = o.write_all(b"\n");
    });
}

/// `print(*parts, sep=sep, end=end, flush=flush)`
pub fn print_kw(parts: &[Str], sep: &str, end: &str, flush_now: bool) {
    with_out(|o| {
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                let _ = o.write_all(sep.as_bytes());
            }
            let _ = o.write_all(p.as_bytes());
        }
        let _ = o.write_all(end.as_bytes());
    });
    if flush_now {
        flush();
    }
}

/// Flush buffered native output (before every CPython call, so output from
/// both sides stays ordered): free when nothing was printed since.
#[inline]
pub fn flush() {
    with_out(|o| {
        if !o.buffer().is_empty() {
            let _ = o.flush();
        }
    });
}

/// Run the compiled module body; report an uncaught exception like CPython
/// (traceback header on stderr, exit status 1).
pub fn run_main(file: &str, body: impl FnOnce() -> PyResult<()>) -> ! {
    let r = body();
    flush();
    let code = match r {
        Ok(()) => 0,
        Err(e) => {
            if !REPORTED.load(std::sync::atomic::Ordering::Relaxed) {
                print_traceback(file, &e);
            }
            1
        }
    };
    finalize_modules();
    flush();
    std::process::exit(code);
}

/// Finalizers of imported native modules: (release function, has a
/// reference cycle), in import order.
static MODULES: crate::Shared<Vec<(fn(u8), bool)>> = crate::Shared::new(Vec::new());

/// An imported native module starts running: register its finalizer.
pub fn register_module(fin: fn(u8), cyclic: bool) {
    MODULES.with(|m| m.push((fin, cyclic)));
}

/// At exit, after the main module: release imported modules' variables as
/// CPython's shutdown does: modules without reference cycles first (freed
/// by reference counting, each in assignment order), then those with cycles
/// in one garbage collection (generators held directly by their variables
/// first, then the rest), each group in import order.
fn finalize_modules() {
    let mods = MODULES.with(std::mem::take);
    for (fin, c) in &mods {
        if !*c {
            fin(2);
        }
    }
    for phase in [0, 1] {
        for (fin, c) in &mods {
            if *c {
                fin(phase);
            }
        }
    }
}

static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn print_traceback(file: &str, e: &crate::Exc) {
    eprintln!("Traceback (most recent call last):");
    eprintln!("  File \"{file}\", in <module>  [native frames elided by pyfast]");
    eprintln!("{e}");
}

/// The main module ended (normally or with an uncaught exception, whose
/// traceback is printed now): its variables are released next, as CPython
/// does at shutdown, so `finally` blocks of open generators run after the
/// traceback.
pub fn module_done(file: &str, r: PyResult<()>) -> PyResult<()> {
    flush();
    if let Err(e) = &r {
        print_traceback(file, e);
        REPORTED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    r
}
