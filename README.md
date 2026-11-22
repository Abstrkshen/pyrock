# pyfast

An ahead-of-time compiler that turns ordinary, unannotated Python into readable Rust. It
infers generic function signatures, specializes each function for the concrete types it is
called with, and builds a native executable with Cargo. It explains every decision it makes.

Status: **prototype, covering milestones M0–M2 of [Project.md](Project.md) plus Rust/C imports.**
In hybrid mode (the default), anything that can't be compiled natively crosses into an
embedded CPython at an explicit, reported boundary. `--mode native-only` rejects those
boundaries instead.

## Imports: Python, Rust and C

A plain `import x` (or `from x import f`) is resolved next to the importing file, in this order:

| Found | What happens |
|---|---|
| `x.py` | Compiled natively as part of the same binary. Its module body runs once, at the first import. If it uses unsupported syntax, hybrid mode imports it through CPython instead and warns (P220). |
| `x.rs` | Linked into the binary with `#[path] mod`. Every `pub fn` whose parameters and return type are integers, `f32`/`f64`, `bool`, `&str`/`String` or `()` is callable. A `Result<T, E>` return raises `RuntimeError(str(e))`. |
| `x.c` | Compiled with the system C compiler (`cc` crate). Signatures come from `x.h` when it exists, otherwise from the non-static definitions in `x.c`. Integer types, `float`/`double`, `bool`, `const char *` and `void` cross the boundary. |
| nothing | A CPython import at runtime: the stdlib, site-packages and C-extension packages (numpy, ...). The build interpreter's `sys.path` is embedded, so a venv works. |

```python
import json            # CPython
import geometry        # geometry.py, compiled natively
import vecmath         # vecmath.rs
import fastc           # fastc.c + fastc.h

print(vecmath.dot(1.5, 2.0, 3.0, 4.0), fastc.gcd(1071, 462))
print(json.dumps({"area": geometry.circle_area(2.0)}))
```

Arguments are converted with checks: a Python int that doesn't fit a C `int` raises
`OverflowError`, and a `str` containing NUL raises `ValueError`. Values returned from Python
(e.g. `json.loads("12")`) can be passed to Rust or C, and they are unboxed with the same
checks. See [examples/imports/](examples/imports/).

## Type annotations

Annotations are optional. Where present, they act as checked type information:

```python
def add(a: int, b: int) -> int:      # add(int, int) -> int
    return a + b

def add2(a, b: int) -> int:          # add2(A, int) -> int where Add(A, int, R): generic in `a` only
    return a + b

def parse(s: str) -> int:
    return json.loads(s)             # the Python object is unboxed to a native i64 here

count: int = json.loads("41")        # native i64 from here on
```

* **Python objects flowing into an annotated slot are unboxed.** This covers arguments,
  `return` values and `x: T = ...`. For `int`, `str` and `bool` the object must be exactly
  that type, otherwise `TypeError`. This is the one place pyfast enforces annotations, because
  converting silently would change behaviour. `float` annotations leave objects boxed: PEP 484
  lets an int stand in for a float, and turning `2` into `2.0` would change the output.
* **Native values keep their runtime type,** as in CPython: `scale(2, 3)` with
  `x: float, k: float` returns the int `6`. A mismatch, such as passing a `str` to `a: int`,
  is warning P307, not an error, because CPython doesn't enforce annotations either.
* `int`, `float`, `str`, `bool`, `None`, `object`/`Any` and string forward references are
  understood. Other annotations (`list[int]`, `Optional[int]`, classes) are accepted and don't
  constrain anything yet.
* `typing`, `typing_extensions` and `__future__` imports used only in annotations are dropped,
  so they never touch CPython.

## The CPython bridge

* Lists, dicts, tuples, method calls, iteration, subscripts, keyword arguments, and builtins
  without a native version (`sorted`, `sum`, `round`, ...) use real Python objects. Mutation
  and aliasing therefore behave exactly as in Python.
* In hybrid mode, a variable or return value that takes several types (`x = 1; x = "s"`, or a
  function that sometimes returns `None`) is boxed as a Python object instead of rejected.
* Python exceptions keep their type and message (e.g.
  `json.decoder.JSONDecodeError: Expecting value...`).
* Native code never holds the interpreter during pure Rust loops. Programs without
  boundaries don't link libpython at all.

```console
$ pyfast explain examples/generics.py
Inferred generic signatures
  add(A, B) -> R where Add(A, B, R)
  twice(A) -> R where Add(A, A, R)
  square(A) -> R where Mul(A, A, R)
  sum_squares(int) -> int
  fib(A) -> A where Lt(A, int), Sub(A, int, A), Add(A, A, A)

Specializations (reachable from module entry)
  twice(x: int) -> int              native  twice__int        effects: may_raise
  twice(x: float) -> float          native  twice__float      effects: pure
  add(a: str, b: str) -> str        native  add__str_str      effects: pure
  ...
$ pyfast run examples/generics.py
```

## Commands

| Command | What it does |
|---|---|
| `pyfast check main.py [--mode native-only]` | Parse, infer and plan; print diagnostics |
| `pyfast explain main.py [--format json]` | Generic signatures, specializations, effects, diagnostics |
| `pyfast emit main.py --out DIR` | Write the Cargo project, source map and report |
| `pyfast build main.py [--release]` | Emit, then `cargo build`; prints the executable path |
| `pyfast run main.py [-- args]` | Build and run |
| `pyfast bench main.py --baseline python` | Median, min and max wall time against CPython, with an output check |

By default, generated projects go in `.pyfast/<name>/` next to the source file. Each one
contains `Cargo.toml`, `src/main.rs`, `pyfast-report.json` and `pyfast.sourcemap.json`.

## How it works

```
Python source ─► CPython ast (JSON protocol v1) ─► scope-resolved HIR
   ─► generic schemes (for explain)    ─► reachability-driven specialization
   ─► eligibility plan + effects       ─► Rust emitter ─► cargo ─► executable
```

| Crate | Role |
|---|---|
| `crates/frontend` | Runs `python3 -I` with [dump_ast.py](crates/frontend/src/dump_ast.py), resolves imports (.py / .rs / .c / CPython), extracts Rust and C signatures, and lowers to HIR |
| `crates/ir` | Spans, diagnostics, HIR, the type lattice |
| `crates/infer` | Operator table, definite assignment, generic schemes, specialization fixed point |
| `crates/analysis` | Call graph, recursion detection, effects, native/bridge tiers, and the CPython boundary list |
| `crates/codegen-rust` | One Rust function per specialization |
| `crates/runtime` | Python semantics in Rust (checked ints, floor/mod, float repr, exceptions), FFI conversions, and the PyO3 bridge (`python` feature) |
| `crates/driver` | The `pyfast` CLI |

## Tests

```console
cargo test --workspace                 # unit + differential + golden + diagnostics
UPDATE_SNAPSHOTS=1 cargo test -p pyfast --test suites   # re-bless snapshots
```

* `tests/differential/*.py`: each program is compiled and run, then compared with CPython
  on stdout, exit status and the final exception line.
* `tests/golden/*.py`: snapshots of the generated Rust and the `explain` output.
* `tests/diagnostics/*.py`: snapshots of `pyfast check` on rejected programs.
* `tests/interop/*/main.py`: programs that import Rust/C modules (CPython can't run these),
  compared against `expected.stdout` / `expected.stderr`.

## Benchmarks

`pyfast bench benchmarks/numeric_loop.py --runs 3` on an Intel Core 7 240H, with Python
3.12.3 and rustc 1.96.0: CPython median 626 ms, native median 13 ms, stdout identical. This is
one integer/float/recursion workload measured with wall-clock time including process startup.
It is not a general speedup claim. Code that spends its time in bridged Python calls
won't be faster, and each boundary crossing has its own cost.

More documentation:

* [docs/imports.md](docs/imports.md): how to import Python, Rust, C and CPython modules
* [docs/how-it-works.md](docs/how-it-works.md): each compiler stage with real generated code
* [docs/subset.md](docs/subset.md): the supported subset, known divergences and diagnostic codes
