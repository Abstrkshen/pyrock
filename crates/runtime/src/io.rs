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
    match r {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("Traceback (most recent call last):");
            eprintln!("  File \"{file}\", in <module>  [native frames elided by pyfast]");
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
