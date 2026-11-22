---
name: pyfast
description: Use pyfast, an ahead-of-time Python → Rust compiler, to compile, run, explain and benchmark Python programs as native executables, and to write Python that imports local .py, .rs and .c files or CPython packages. Use when a task mentions `pyfast`, a `.pyfast/` folder, compiling or speeding up a Python script with Rust, calling Rust or C from Python without bindings, a diagnostic code like P143/P215/P301, `--mode native-only`, or changing the pyfast compiler repo itself.
---

# Using pyfast

pyfast compiles ordinary, unannotated Python into readable Rust, builds it with Cargo and
produces a native executable. It infers the types of every function from how it is called,
and compiles one specialized copy per type combination (`add(1, 2)` → `add__int_int`).
Anything it can't compile natively runs in an embedded CPython at an explicit, reported
**boundary** (hybrid mode, the default). `--mode native-only` turns boundaries into errors.

`pyfast` is on PATH (installed by `bman` into `~/Documents/bin`). Check with `pyfast --version`.
The source repo is `~/Documents/projects/pyfast`. The binary carries its runtime crate and
extracts it to `~/.cache/pyfast/runtime/<hash>/` on first use, so it works from anywhere
(`PYFAST_RUNTIME` overrides it with a runtime crate path).

## Commands

```sh
pyfast run main.py [--release] [--mode native-only] [-- args...]   # build + run
pyfast build main.py [--release] [--out DIR]   # prints the executable path
pyfast check main.py [--mode native-only]      # diagnostics only, no build (fast)
pyfast explain main.py [--format json]         # imports, inferred generics, specializations, boundaries
pyfast emit main.py --out DIR                  # write the Cargo project, don't build
pyfast bench main.py --baseline python --runs 5  # wall-time vs CPython + output check
```

Global option: `--python PATH` selects the interpreter used for parsing, for embedding and
for `sys.path` (use a venv's python to get its packages).

Output layout: `.pyfast/<stem>/` next to the source holds `Cargo.toml`, `src/main.rs`,
`pyfast-report.json`, `pyfast.sourcemap.json` and, after `build`/`run`, the executable
`.pyfast/<stem>/<stem>`. Cargo artifacts go in the shared cache `~/.cache/pyfast/target`
(`PYFAST_TARGET_DIR` overrides). The first build anywhere compiles the runtime and PyO3,
which takes a minute or two. Later builds are fast.

Requirements: `cargo` on PATH (in a non-login agent shell, run `export PATH=$HOME/.cargo/bin:$PATH`
first), `python3` with `libpython3.x.so` if the program uses CPython, and a C compiler if it
imports `.c` files.

## Recommended workflow

1. `pyfast check file.py`. Fix every `error` (see [references/diagnostics.md](references/diagnostics.md)).
2. `pyfast explain file.py`. Confirm which imports resolved where, and read the "CPython
   boundaries" list. Every listed line runs through Python at runtime.
3. `pyfast run file.py`, then compare with `python3 file.py`. Output is meant to be identical,
   including the final `Kind: message` line of an uncaught exception and exit status 1.
4. For speed, use `--release` and remove boundaries from hot loops. A program that passes
   `pyfast check --mode native-only` never touches CPython and doesn't link libpython.

## Imports in one table

`import x` / `from x import f` looks in the importing file's directory, in this order:

| Found | Result |
|---|---|
| `x.py` | Compiled natively into the same binary. Functions are specialized per call types, and the module body runs once. If it can't be compiled (a class with a CPython base class, a metaclass, `match`, ...) it falls back to CPython with warning P220 |
| `x.rs` | `#[path] mod`. `pub fn` with scalars, strings, `Vec`/slices (`&mut` written back), `Option`, tuples, `HashMap`, `impl Fn` callbacks, `pub struct` handles with methods, `Result` (Err → `RuntimeError`); crates via `// pyfast: dep` |
| `x.c` (+`x.h`) | Built with `cc`, called over FFI. Scalars, strings, pointer+length arrays, `typedef struct` values, function pointers; `// pyfast: link` / `pkg-config` |
| nothing | CPython import at runtime (stdlib, site-packages, numpy, ...) |

Dotted names (`import os.path`) always go to CPython. Relative imports, `import *` and imports
inside functions are rejected. Full details and type tables are in [references/imports.md](references/imports.md).

## What compiles natively (short version)

Native means plain Rust with no Python objects: functions (recursion, defaults, keyword
args, `*args`/`**kwargs`), int (i64, overflow raises `OverflowError`), float, str, bool, None,
`T | None`, all operators, lists/dicts/sets/tuples with their methods, comprehensions,
unpacking, string methods and formatting, most builtins, `try`/`except`/`raise`, classes
(properties, dataclasses, inheritance, dunders, user exceptions), lambdas, closures,
decorators, module globals, generators (`yield`), and `async def`/`await` run by asyncio.

Bridged through CPython (hybrid only): anything touching an object from a CPython import
(stdlib, site-packages), `with` on Python context managers, `await` of Python awaitables,
and variables or returns with mixed types (boxed).

Not supported in either mode: classes with a CPython base class or a metaclass (keep them in
an imported `.py`, which falls back to CPython),
relative imports, `import *`.

Not supported either: `match`, `:=`, `for`/`while ... else`, `del name`, imports inside
functions, non-literal defaults of top-level functions, `Enum`/`NamedTuple` subclasses,
multiple inheritance, method decorators other than `@property`/`@staticmethod`/`@classmethod`.
On the standard library as it is, 54% of functions are accepted (`docs/coverage.md` in the repo).

See [references/language.md](references/language.md) for the precise subset, semantics and
the rewrites that make code compile.

## Writing Python for pyfast: key rules

* Put logic in top-level functions. Module-level variables can be read (and, with `global`,
  assigned) inside functions; they are typed global cells.
* Keep each variable to one type. `x = 0` then `x = "a"` boxes it (hybrid) or is P304
  (native-only). `int`/`float` mixes (`x = 0; x += 0.5`) stay native as `int | float`.
* Dicts with literal string keys (`{"x": 1.0, "hp": 3}`) compile to structs with one type
  per key; using them like generic dicts (`d[k]` with a variable key, `.items()`) keeps one
  value type for all keys instead.
* Return one type on every path. `None` on some paths is a native `T | None`; other mixes
  box the result (P215).
* Define a function before the module-level code that calls it (P144).
* Annotations are optional. `def f(a: int) -> str` is understood, and an `int`/`str`/`bool`
  annotation unboxes a Python object (e.g. `json.loads(...)`) into a native value, with a
  checked `TypeError`.
* Hot loops should use native values (ints, floats, strings, native lists/dicts, instances)
  and call native or Rust/C functions. Each CPython call in a loop costs a bridge crossing.

## Changing the pyfast compiler itself

See [references/developing.md](references/developing.md) for the crate map, test suites,
snapshot re-blessing and the `perch` commands (`perch install` rebuilds and reinstalls the
global binary).
