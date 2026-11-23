# Current state (2026-11-23)

What pyfast compiles today, how fast it is, how it is tested, and what is still missing.
The step-by-step history is in [roadmap.md](roadmap.md); the precise language subset and the
diagnostic codes are in [subset.md](subset.md).

## In one paragraph

pyfast compiles ordinary Python into a native executable through readable Rust. Code it can
type compiles to plain Rust; anything else (imported Python packages, values without a
single type) runs through an embedded CPython at reported boundaries (hybrid mode, the
default). `--mode native-only` turns those boundaries into errors, and such programs don't
link libpython at all. Rust (`.rs`) and C (`.c`) files next to the program are imported
like Python modules, without bindings. Output matches CPython: every feature is tested by
running the same program under both and comparing stdout, exit status and the final
exception line.

## How much of Python

52 of the 55 statement and expression kinds of Python 3.12 compile natively. The missing
ones are `match`, `type X = ...` and `except*`. Measured on code as it is, 62% of the
standard library's functions and methods are accepted, and 68%, 73% and 41% of those in the
`requests`, `huggingface_hub` and `langchain_core` packages (each measured in place, with
their relative imports). The main gaps left are other decorators (`functools`,
library-specific ones), CPython base classes (`NamedTuple`, `pydantic`, `Protocol`), mixins
and computed class-body assignments.
Details and the ranked blockers: [coverage.md](coverage.md).

## What compiles natively

| Area | Supported |
|---|---|
| Values | `int` (64-bit, overflow raises `OverflowError`), `float`, `str`, `bool`, `None`, `T \| None`, `int \| float` (a variable that is an int on some paths and a float on others) |
| Collections | `list`, `dict`, `set`, tuples, with their methods, slicing, unpacking, comprehensions; dicts with literal string keys become structs with a type per key (keys added later included) |
| Functions | defaults, keyword arguments, `*args`/`**kwargs`, recursion, lambdas, closures, `nonlocal`, decorators, functions as values, module globals |
| Classes | methods, properties, `@staticmethod`/`@classmethod`, dunders, `@dataclass`, single inheritance, user exceptions, unions of classes |
| Control flow | `if`/`while`/`for`, `try`/`except`/`else`/`finally`, `raise`, `with` (Python context managers), `assert` |
| Generators | `yield`, `yield from` (with delegation and return values), `send`/`throw`/`close`, lazy generator expressions, `finally` when abandoned or at exit |
| Async | `async def`/`await` run by asyncio, `async with`, `async for`, async generators (`asend`/`athrow`/`aclose`), async comprehensions |
| Stdlib | `math` natively (CPython's errors); other modules through CPython |
| Formatting | f-strings with format specs, `%`, `str.format`, `print` with `sep`/`end` |

Native values passed to CPython stay usable there: functions become Python callables,
instances become Python objects (fields, methods, identity), generators and coroutines
become Python iterators and awaitables. Native functions can be called from CPython
threads.

## Rust and C imports

* Rust: scalars, strings, `Vec`/slices (`&mut` changes are written back, including into
  `array.array`/numpy arrays), `Vec<u8>` as `bytes`, `Option`, tuples, `HashMap`,
  callbacks (`impl Fn`, `&dyn Fn`) taking Python functions/lambdas/closures, `pub struct`
  handles with `impl` methods and fields, `Result` (errors raise). Crates via a
  `// pyfast: dep NAME = "VERSION"` comment.
* C: scalars, strings, pointer+length arrays (written back when not `const`),
  `typedef struct` values, function pointers (Python functions as C callbacks),
  `// pyfast: link NAME` and `// pyfast: pkg-config NAME`.

Full type tables: [imports.md](imports.md).

## Speed (measured, `--release`, Intel Core 7 240H, CPython 3.12.3)

| Program | CPython | pyfast | |
|---|---|---|---|
| `benchmarks/numeric_loop.py` (int/float/recursion) | 626 ms | 13 ms | 48x |
| `demos/gui/headless.py` (300 balls, pairwise collisions) | 3.3 s | 0.2 s | 16.7x |
| Game state in dicts (200k frames, record dicts) | 0.58 s | 31 ms | 18.8x |
| The same state in classes | 0.47 s | 46 ms | 10x |
| `demos/ai/bench_bpe.py` (string-heavy tokenizer training) | 0.40 s | 0.15 s | 2.7x |

Code that mostly calls into CPython (numpy, OpenCV, the `raylib` package) gains little:
each bridged operation costs about 15 ns against about 5 ns in CPython's own specialized
bytecode, so keep hot loops on native values.

## Demos and samples

* [demos/](../demos/): a JSON API on asyncio (backend), a tokenizer + Rust model + C
  sampling program that needs no CPython (AI), and a raylib ball simulation (GUI).
* `samples/` (21 programs) and the user samples in `.ignore/samples/` (16 README checks),
  including `raylib-breakout`, a full platformer: 55 native functions, the rest is the
  raylib API and ctypes code.

## How it is tested

```console
cargo test --workspace         # differential (vs CPython), golden (generated Rust), diagnostics, interop
bash samples/run_all.sh        # 21 samples vs CPython
bash demos/run_all.sh          # the 3 demos vs expected output
bash scripts/readme_check.sh   # every README command, verbatim
```

All pass. CI ([.github/workflows/ci.yml](../.github/workflows/ci.yml)) runs the same,
with the GUI demo under a virtual display.

## Installation

`perch install` builds a release binary into `~/Documents/bin/pyfast`. The binary carries
its runtime crate and unpacks it into `~/.cache/pyfast/runtime/<hash>/` on first use, so it
does not depend on the repository's location. Generated projects go in `.pyfast/<name>/`
next to the program; compiled dependencies are cached in `~/.cache/pyfast/target`.

## Known differences from CPython

* Integers are 64-bit: results beyond that raise `OverflowError` (CPython promotes to a big
  integer).
* Tracebacks show the header and the final `Kind: message` line, not the frames.
* Set iteration order is insertion order (CPython's depends on hashes).
* `RecursionError` counts only recursive native frames (limit 1000).
* CPython threads run only while compiled code waits (I/O, `sleep`, `join`, the asyncio
  event loop), not during long native loops (the GIL is held while native code runs).
* At exit, CPython's own order of finalizing several imported modules that define
  functions is not stable (it changes once `.pyc` files are cached); pyfast uses one fixed
  order.

## Not supported yet

* Classes with a CPython base class (`NamedTuple`, `dict`, ...), multiple inheritance,
  metaclasses, class decorators other than `@dataclass`, method decorators other than
  `@property`/`@staticmethod`/`@classmethod` (a module using them is imported through
  CPython instead, warning P220).
* `match`, `type X = ...`, `except*`, complex numbers.
* `f(*xs)` where `xs` is a list filling fixed parameters.
* `import *`.
* Arbitrary-precision integers.
* `globals()`, `locals()`, `vars()`/`dir()` without arguments, and `eval`/`exec` without an
  explicit namespace dict (compiled code has no Python frame; a module using them is imported
  through CPython).

## Compared with Project.md

[Project.md](../Project.md) is the original design. Milestones M0–M4 and the M6 subset
(collections, classes, closures, callbacks) are done, plus async, generators and rich
Rust/C interop beyond it. Where the implementation differs:

* Integers are checked 64-bit, the "documented fixed-width subset" option of §6, not
  arbitrary precision.
* There is no `strict` mode (§4); `--mode native-only` covers rejecting every boundary.
* There is no separate block/SSA IR with a verifier (§9): specialization works on the HIR,
  and rustc checks the emitted code.
* The GIL is held while native code runs (§8 asks to release it during long computations).
* The CLI is `pyfast`, not `pyrust`; `emit` has no `--target` (Rust is the only backend).
* Benchmarks measure wall time and output only, on Linux only (§12 also lists compile time,
  startup, RSS, code size, bridge overhead and macOS).

These are tracked as phase 8 in [roadmap.md](roadmap.md).
