# Changelog

The release workflow puts a version's section at the top of its GitHub release notes.

## Unreleased

- `pyrock build --target wasm32-wasip1` compiles a native-only program to WebAssembly, for
  any WASI runtime. Generators aren't supported there, because WebAssembly can't switch
  stacks: creating one raises `RuntimeError`.
- The playground's Run starts the WebAssembly build pyrock made of the example, then runs
  the same source in CPython (Pyodide) and shows both times and whether the outputs match.
  Before, Run only used CPython. Code you edit still runs in CPython, since building needs
  Rust's compiler, and the page says so.

## v0.1.2 (2026-11-23)

### Speed

- Generator expressions passed to `tuple`, `list`, `set`, `frozenset`, `dict`, `sorted`,
  `sum`, `min`, `max` or `"...".join` compile to plain loops. Before, each one ran as a
  coroutine with its own 2 MiB stack, costing about 5 µs per call. The BPE tokenizer
  benchmark goes from 0.23x to 3.0x CPython.
- Other generators reuse coroutine stacks instead of mapping a new one each time.
- Every speed figure in the docs was re-measured at a named commit. Run
  `scripts/bench_all.sh` to repeat the measurements. CI now fails when a speedup drops well
  below its documented figure.

### Fixes

- `any()` and `all()` over a generator stop at the first item that decides the result, as
  in CPython. Before, they read the whole generator, so an infinite one hung.
- `isinstance` narrows a variable that can hold several classes: inside
  `if isinstance(v, A):`, `v.x` and `v.method()` use class `A`. This covers `elif`, `not`,
  `and`/`or`, `while`, conditional expressions and tuples of classes.
- Programs no longer fall back to Python objects everywhere in these cases:
  - `zip(obj.items, args)` in a recursive function;
  - methods overridden across a class hierarchy;
  - a recursive call inside a comprehension, such as
    `" ".join([c.show() for c in self.kids])`.
- `for x in []` compiles in more places.

### Command line

- `pyrock bench` reports a speedup only when both runs exit 0 and print the same output.
  Otherwise it fails and shows the failing run's error output.
- `bench --min-speedup X` fails when the speedup is below X, for use in CI.
- `bench` prints the CPU model only with `--hardware`.
- `--help` and `-h` print the usage and exit 0, after any command. An unknown command is
  reported as such.
- `explain` shows file paths relative to the current directory. A long record-type
  signature no longer widens every row of the table.

### Executables

- Source paths compiled into executables show your home directory as `~`.
- `PYROCK_SYS_PATH` replaces a hybrid executable's built-in `sys.path` at run time, so it
  can use another installation of the same Python version. The README explains what a
  hybrid executable needs on another machine.
- macOS: hybrid programs built with Xcode's `/usr/bin/python3` start. Before, they failed
  with `dyld: Library not loaded: @rpath/Python3.framework`.

### Website and examples

- The playground runs the compiler in the browser and highlights Python syntax. It draws
  programs that print SVG, HTML or PPM images, including animations.
- New playground programs: a ray tracer, a Lisp interpreter, maze search, flocking birds,
  double pendulums and an L-system plant.
- Examples live in one `examples/` tree: `tour`, `interop`, `packages`, `apps`, `graphics`
  and `playground`.

## v0.1.1 (2026-11-23)

- First published binaries: Linux x86-64 and arm64 (static), and macOS on Apple silicon,
  each smoke-tested before publishing; `install.sh`.

## v0.1.0 (2026-11-23)

- Tagged; the macOS build did not run, so v0.1.1 is the first complete release.
