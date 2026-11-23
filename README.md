# pyrock

An ahead-of-time compiler that turns ordinary, unannotated Python into readable Rust. It
infers generic function signatures, specializes each function for the concrete types it is
called with, and builds a native executable with Cargo. It explains every decision it makes.

Website: **https://abstrkshen.github.io/pyrock/**, with a [playground](https://abstrkshen.github.io/pyrock/play.html)
that runs the compiler in your browser. Releases: [github.com/Abstrkshen/pyrock/releases](https://github.com/Abstrkshen/pyrock/releases)

Status: **prototype.** Classes, closures, exceptions, native collections, generators and
async/await compile natively, as does Rust and C interop with structs, slices and callbacks.
52 of Python's 55 statement and expression kinds compile; on the standard library as it is,
64% of functions and methods are accepted ([docs/coverage.md](docs/coverage.md)).
The current state, with measured speedups and known differences, is in
[docs/status.md](docs/status.md); the history is in [docs/roadmap.md](docs/roadmap.md).
In hybrid mode (the default), anything that can't be compiled natively crosses into an
embedded CPython at an explicit, reported boundary. `--mode native-only` rejects those
boundaries instead.

## Installation

pyrock compiles your program to Rust and builds it with Cargo, so besides the `pyrock`
command it needs:

* a Rust toolchain ([rustup](https://rustup.rs); tested with Rust 1.96);
* Python 3 (tested with 3.12): pyrock parses with CPython's own `ast` module, and programs
  that use Python packages embed it, which needs the shared `libpython` (Debian/Ubuntu:
  `sudo apt install python3-dev`);
* a C compiler (`cc`) if your programs import `.c` files.

**Prebuilt binary** (Linux x86-64 and arm64, macOS on Apple silicon):

```sh
curl -fsSL https://raw.githubusercontent.com/Abstrkshen/pyrock/main/install.sh | sh
```

The script downloads the archive for your platform from the
[releases](https://github.com/Abstrkshen/pyrock/releases), checks its SHA-256 and installs
`pyrock` into `~/.local/bin` (`PYROCK_INSTALL_DIR` changes that, `PYROCK_VERSION=v0.1.0`
picks a release). You can also download an archive from the releases page yourself. The
Linux binaries are static and run on any distribution.

**From source** (any platform with Rust; tested on Ubuntu 24.04):

```sh
cargo install --git https://github.com/Abstrkshen/pyrock pyrock
# or, from a clone:
git clone https://github.com/Abstrkshen/pyrock.git && cd pyrock
cargo install --path crates/driver     # installs ~/.cargo/bin/pyrock
```

Check it with `pyrock --version`. The binary carries its runtime crate, so nothing else
from the repository is needed. The first build of any program compiles the runtime (and
PyO3 if needed) into `~/.cache/pyrock/target`, which takes a minute or two; later builds
take seconds.

A first program:

```sh
cat > fib.py <<'EOF'
def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)

print(fib(30))
EOF
pyrock run fib.py            # build and run (the executable is .pyrock/fib/fib)
pyrock explain fib.py        # what was inferred and compiled natively
pyrock check fib.py --mode native-only
```

The [examples](examples/) show Rust and C imports, Python packages and larger programs.
Optional: `bash skills/install.sh` links an agent skill describing pyrock into Claude
Code, Codex and Cursor.

## Usage

```sh
pyrock check   main.py [--mode native-only]          # diagnostics only, no build (fast)
pyrock explain main.py [--format json]               # imports, inferred types, boundaries
pyrock run     main.py [--release] [--mode native-only] [-- ARGS...]
pyrock build   main.py [--release] [--out DIR]       # prints the executable's path
pyrock emit    main.py [--out DIR]                   # write the Cargo project, don't build
pyrock bench   main.py [--runs N] [--min-speedup X]  # time against CPython
```

`main.py` is the program's entry file; the `.py`, `.rs` and `.c` files it imports from its
directory are compiled into the same executable. A typical session:

1. **`pyrock check main.py`** reports what can't compile, with a code and a reason for each
   problem (`main.py:12:5  P301  error: ...`). It doesn't build anything, so it is quick.
2. **`pyrock explain main.py`** shows how each import was resolved (Python, Rust, C or
   CPython), the generic signature inferred for each function, every specialization that was
   compiled (`native  add(a: int, b: int) -> int  add__int_int`), and the list of
   **CPython boundaries**: each line of your program that will run through Python.
3. **`pyrock run main.py`** builds and runs. Arguments after `--` go to your program. Output,
   exit status and the final `Kind: message` line of an uncaught exception match `python3`.
4. **`pyrock run main.py --release`** turns on optimizations; use it to measure speed.
   **`pyrock bench main.py`** times the program against CPython. It reports a speedup only
   when both runs exit 0 and print the same output, and fails otherwise.

Options:

| Option | Effect |
|---|---|
| `--mode hybrid` (default) | Code that can't be compiled natively (Python packages, values without a single type) runs in an embedded CPython at a reported boundary |
| `--mode native-only` | Every boundary is an error instead. The executable doesn't link libpython and needs no Python to run |
| `--release` | Optimized build (Cargo's release profile) |
| `--python PATH` | The interpreter used to parse, to embed, and whose `sys.path` the program sees. Pass a virtualenv's `python` to use its packages |
| `--out DIR` | Where `emit`/`build` write the Cargo project (default `.pyrock/<name>/` next to the source) |
| `--format json` | `explain` as JSON, for tools |
| `--runs N` | `bench`: runs of each program (default 5); the median is reported |
| `--min-speedup X` | `bench`: exit 1 unless the native build is at least X times faster (for CI) |
| `--hardware` | `bench`: also print the CPU model |

**What you get.** `.pyrock/<name>/` holds the generated `Cargo.toml` and `src/main.rs`
(readable Rust, one function per specialization, each with a comment pointing at its
Python line), `pyrock-report.json`, `pyrock.sourcemap.json` and, after a build, the
executable `.pyrock/<name>/<name>`. A native-only executable is self-contained: copy it to
another machine with the same OS and architecture and run it.

**Hybrid executables need the build machine's Python.** A hybrid executable loads
`libpython` of the same Python version it was built with, and starts CPython with the
build interpreter's `sys.path` (its standard library, its virtualenv's site-packages, the
source folder) compiled in. It runs as is where that Python and those packages are at the
same paths. Elsewhere, set `PYROCK_SYS_PATH` (a `:`-separated list that replaces the
compiled-in one) and, if the standard library lives elsewhere, `PYTHONHOME`. Compiled
modules keep their source path as `__file__`, as in CPython. Other source paths compiled
into a binary (in panic messages) show your home directory as `~`.

**Diagnostics.** Codes are grouped by kind: `P0xx` the source couldn't be read, `P1xx` a
construct pyrock doesn't support or a name problem, `P2xx` a CPython boundary (errors only
in native-only mode), `P3xx` types (`P301` is an operation CPython would reject with
`TypeError`). The full table is in [docs/subset.md](docs/subset.md#rejected-at-compile-time-diagnostic-codes).

**Environment.** `PYROCK_TARGET_DIR` moves the shared Cargo build cache (default
`~/.cache/pyrock/target`); `PYROCK_RUNTIME` points at a runtime crate to use instead of the
embedded one. At run time, `PYROCK_SYS_PATH` replaces a hybrid executable's compiled-in
`sys.path`.

## Imports: Python, Rust and C

A plain `import x` (or `from x import f`) is resolved next to the importing file, in this order:

| Found | What happens |
|---|---|
| `x.py` | Compiled natively as part of the same binary (functions, classes, module variables). Its module body runs once, at the first import. If it uses unsupported syntax, hybrid mode imports it through CPython instead and warns (P220). |
| `x.rs` | Linked into the binary with `#[path] mod`. `pub fn` with scalars, strings, `Vec`/slices (`&mut` written back), `Option`, tuples, `HashMap`, callbacks (`impl Fn`), `pub struct` handles with methods, and `Result` (errors raise). Crates via `// pyrock: dep`. |
| `x.c` | Compiled with the system C compiler (`cc` crate). Signatures come from `x.h` when it exists, otherwise from the non-static definitions in `x.c`. Scalars, strings, pointer+length arrays, `typedef struct` values and function pointers cross the boundary; `// pyrock: link` / `pkg-config` add libraries. |
| `x/__init__.py` | A package: its modules compile natively, with `import x.y`, `from x import y` and relative imports (`from .y import f`) inside it. |
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
checks. See [examples/interop/imports/](examples/interop/imports/).

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
  that type, otherwise `TypeError`. This is the one place pyrock enforces annotations, because
  converting silently would change behaviour. `float` annotations leave objects boxed: PEP 484
  lets an int stand in for a float, and turning `2` into `2.0` would change the output.
* **Native values keep their runtime type,** as in CPython: `scale(2, 3)` with
  `x: float, k: float` returns the int `6`. A mismatch, such as passing a `str` to `a: int`,
  is warning P307, not an error, because CPython doesn't enforce annotations either.
* `int`, `float`, `str`, `bool`, `None`, `object`/`Any` and string forward references are
  understood. On variables, container annotations (`parts: list[str] = []`,
  `REGISTRY: dict[str, Agent] = {}`, `best: Optional[str] = None`) give a still-empty
  container its element types. Other annotations are accepted and don't constrain anything.
* `typing`, `typing_extensions` and `__future__` imports used only in annotations are dropped,
  so they never touch CPython.

## The CPython bridge

* Values from CPython imports (stdlib, site-packages, numpy, ...) are Python objects; every
  operation on them goes through CPython, so they behave exactly as in Python.
* In hybrid mode, a variable that takes types pyrock can't unify (`x = 1; x = "s"`) is
  boxed as a Python object instead of rejected. (`int`/`float` mixes, `T | None` and dicts
  with fixed string keys stay native.)
* Native values passed to CPython keep working there: functions become callables, instances
  become objects, generators and coroutines become iterators and awaitables. Native
  functions can be called from CPython threads.
* Python exceptions keep their type and message (e.g.
  `json.decoder.JSONDecodeError: Expecting value...`).
* A program that embeds CPython holds the GIL while native code runs; CPython threads run
  while it waits. Programs without boundaries don't link libpython at all.

```console
$ pyrock explain examples/tour/06_generics.py
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
$ pyrock run examples/tour/06_generics.py
```

## Examples

Everything runnable is in [examples/](examples/), grouped by what it shows; its README is
the index and covers setup:

| Folder | What's in it |
|---|---|
| [tour/](examples/tour/) | 23 small numbered programs, one idea each, checked against CPython |
| [interop/](examples/interop/) | Importing `.rs`, `.c` and `.py` files, and CPython modules |
| [packages/](examples/packages/) | pip packages (`huggingface_hub`, LangChain) through CPython |
| [apps/](examples/apps/) | A JSON API on asyncio, an AI pipeline with no CPython (tokenizer in Python, model in Rust, sampling in C), a raylib physics sim, an AI-agent server |
| [graphics/](examples/graphics/) | raylib windows, a platformer, live YOLO vision with OpenCV and ONNX Runtime |
| [playground/](examples/playground/) | The [browser playground](https://abstrkshen.github.io/pyrock/play.html)'s programs, including pictures and animations drawn from printed SVG, HTML and PPM |

Speed measurements are in [benchmarks/](benchmarks/) and [docs/status.md](docs/status.md).

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
| `crates/runtime` | Python semantics in Rust (checked ints, floor/mod, float repr, exceptions, `int \| float`), collections, generators and coroutines (stackful, `corosensei`), `math`, FFI conversions, and the PyO3 bridge (`python` feature). Embedded in the `pyrock` binary |
| `crates/driver` | The `pyrock` CLI |
| `crates/playground` | The compiler as a WebAssembly module for the website's playground: takes `dump_ast.py`'s JSON (parsed by Pyodide in the browser) and returns `check`, `explain` and the generated Rust |

## Tests

```console
cargo test --workspace                 # unit + differential + golden + diagnostics
UPDATE_SNAPSHOTS=1 cargo test -p pyrock --test suites   # re-bless snapshots
bash examples/tour/run_all.sh          # every tour program vs CPython
bash examples/apps/run_all.sh          # the apps vs their expected output
bash scripts/readme_check.sh           # every example README's commands, verbatim
bash scripts/build_site.sh             # the website's playground (compiler as wasm, programs)
```

Progress towards compiling real backend, AI and GUI apps is tracked in
[docs/roadmap.md](docs/roadmap.md).

* `tests/differential/*.py`: each program is compiled and run, then compared with CPython
  on stdout, exit status and the final exception line.
* `tests/golden/*.py`: snapshots of the generated Rust and the `explain` output.
* `tests/diagnostics/*.py`: snapshots of `pyrock check` on rejected programs.
* `tests/interop/*/main.py`: programs that import Rust/C modules (CPython can't run these),
  compared against `expected.stdout` / `expected.stderr`.

## Benchmarks

`bash scripts/bench_all.sh` measures each benchmark against CPython with `pyrock bench`
and prints a table for the current commit. At commit `1c69939`, on an Intel Core 7 240H
with Python 3.12.3:

| Program | Speedup |
|---|---|
| `benchmarks/numeric_loop.py` (int and float loops, recursion) | 47.9x |
| `benchmarks/game_state_dicts.py` (game state in dicts) | 17.7x |
| `examples/apps/gui/headless.py` (300 balls, pairwise collisions) | 16.3x |
| `benchmarks/game_state_classes.py` (the same state in classes) | 11.8x |
| `examples/apps/ai/bench_bpe.py` (BPE tokenizer training, string-heavy) | 3.0x |
| `benchmarks/collections_bench.py` (word counts, sorting, a prime sieve) | 3.0x |

Times are in [docs/status.md](docs/status.md). They are wall-clock times including process
start, not a general speedup claim. Code that spends its time in Python packages
(LangChain, `huggingface_hub`, numpy) gives the same output but is no faster, since that
work still runs in CPython, and its executable needs the build machine's virtualenv. CI
fails when a speedup drops well below these figures.

More documentation:

* [docs/status.md](docs/status.md): the current state, measured speedups, known differences
* [docs/coverage.md](docs/coverage.md): how much of Python compiles, by construct and measured on real code
* [docs/imports.md](docs/imports.md): how to import Python, Rust, C and CPython modules
* [docs/how-it-works.md](docs/how-it-works.md): each compiler stage with real generated code
* [docs/subset.md](docs/subset.md): the supported subset, known divergences and diagnostic codes
