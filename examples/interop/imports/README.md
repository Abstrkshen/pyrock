# imports

Every kind of import in one program: `geometry.py` compiled natively, `vecmath.rs`
(Rust) and `fastc.c` with `fastc.h` (C) linked in without bindings, and `json` from
CPython. Needs a C compiler. [docs/imports.md](../../../docs/imports.md) walks through it.

```sh
pyrock explain main.py
pyrock run main.py
```

In `pyrock explain`, the "Imports" section shows where each name resolved and the Rust
and C signatures pyrock read.
