//! Output and process entry. stdout is block-buffered (like CPython when
//! not attached to a terminal) and flushed before exit or a traceback.

use crate::exc::PyResult;
use crate::Str;
use std::cell::RefCell;
use std::io::{BufWriter, Write};

thread_local! {
    static OUT: RefCell<BufWriter<std::io::Stdout>> = RefCell::new(BufWriter::with_capacity(1 << 16, std::io::stdout()));
}

/// `print(*parts)` with the default `sep=" "` and `end="\n"`.
pub fn print(parts: &[Str]) {
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                let _ = o.write_all(b" ");
            }
            let _ = o.write_all(p.as_bytes());
        }
        let _ = o.write_all(b"\n");
    });
}

pub fn flush() {
    OUT.with(|o| {
        let _ = o.borrow_mut().flush();
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
