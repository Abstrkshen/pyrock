# Supported subset

## Compiled natively

These constructs become plain Rust with no Python objects involved:

* Top-level functions, including direct and mutual recursion. Parameters may be positional-only
  (`/`), positional-or-keyword or keyword-only (`*`), with literal defaults (numbers, strings,
  `None`, booleans, tuples of those). Calls may pass keyword arguments; arguments are still
  evaluated in source order.
* `T | None`: a value that is `None` on some paths and a `T` on others is a native
  `Option<T>` (no boxing, also in native-only mode). Using it where a `T` is needed checks it
  and raises CPython's `TypeError` on `None`. `x is None` on a value whose type is exactly
  `None` (or can never be `None`) in a specialization is decided at compile time, so
  `if size is None: size = 10` with a `None` default compiles to straight-line code.
* Local variables. Each local has one type per specialization.
* Literals: `int` (i64 range), `float`, `str`, `bool`, `None`.
* Operators: `+ - * / // % ** & | ^ << >> ~`, unary `-`/`+`/`not`, and comparisons including
  chains (`a < b <= c`).
* `and`/`or` return their deciding operand, as in Python. `x if c else y`.
* `if/elif/else`, `while`, `for x in range(...)` with 1 to 3 arguments, `break`, `continue`,
  `pass`, `return`, augmented assignment, and chained assignment (`a = b = 1`).
* Builtins: `print(*args, sep=, end=, flush=)`, `abs`, `int`, `float`, `str`, `bool`, `len(str)`, and
  `min`/`max` with 2 or more arguments of the same type.
* f-strings with plain `{expr}` placeholders.
* Exceptions: `try`/`except`/`else`/`finally` (including `except (A, B) as e`, bare `except:`
  and bare `raise`), `raise E`, `raise E(msg)`, `raise e`, `assert cond[, msg]`, and builtin
  exception values such as `ValueError("x")`. `except` matches through CPython's builtin
  exception hierarchy (`except LookupError` catches `KeyError`); exceptions that come from
  CPython keep their original object, so `except json.JSONDecodeError` works through
  `isinstance`. `return`/`break`/`continue` inside `try` run `finally` first.
* `__name__` and `__file__`.
* Module-level variables read inside functions, `global x` in functions, `mod.NAME` /
  `mod.NAME = v` on a natively compiled module, and `from mod import NAME` (copied at import,
  as CPython does). Such variables live in typed global cells; reading one before its
  assignment ran raises `NameError`.

## Semantics that are preserved

* Floor division and modulo signs, true division (including correctly rounded `int / int`),
  `float` floor/mod/pow special cases, and exact `int`-vs-`float` comparison.
* `repr(float)` formatting (`1e+16`, `1e-05`, `-0.0`, `inf`, `nan`).
* `bool` is an `int` in arithmetic, but prints as `True`/`False`; `True & False` is a `bool`.
* Exceptions: `ZeroDivisionError`, `ValueError`, `OverflowError`, `UnboundLocalError`,
  `NameError` and `RecursionError` (limit 1000, enforced only in recursive functions). An
  uncaught exception exits with status 1 and prints `Kind: message` as the last stderr line.
* Locals that may be unbound are stored as `Option<T>` and raise exactly where CPython would.

## Known divergences (documented, tested where possible)

| Behaviour | CPython | pyfast M1 | Planned fix |
|---|---|---|---|
| int results outside i64 | promotes to bigint | `OverflowError` | M4: arbitrary precision / promotion |
| `int ** negative_int` with a non-literal exponent | float | `NotImplementedError` | M4: union return |
| negative float `**` fractional power | complex | `NotImplementedError` | out of scope |
| Traceback body | full frames | last line only | source-map-driven tracebacks |
| `RecursionError` threshold | counts every frame | counts recursive frames only | — |
| Python object returned into an `int`/`str`/`bool` annotation it doesn't match | annotation ignored | `TypeError` at the unbox | — (deliberate: unboxing must not change the value) |

## Bridged through CPython (hybrid mode)

These are compiled, but they operate on `rt::py::Obj` values through the embedded interpreter.
Each site appears under "CPython boundaries" in `pyfast explain`, and each is a compile error
under `--mode native-only`.

* `import x` / `from x import y`, where no `x.py`, `x.rs` or `x.c` exists next to the file (P200)
* Attribute access, method calls, calls of Python objects, keyword arguments (P210)
* Builtins without a native implementation, e.g. `sorted`, `sum`, `round`, `isinstance`, `repr` (P210/P211)
* List, dict, tuple and set literals, subscripts and slices, item and attribute assignment (P212)
* Operators and comparisons involving a Python object (P213), and `for x in <iterable>` (P214)
* Variables or returns with no single native type (P215: boxed)
* `with` statements (CPython context-manager protocol, P216), and `except` clauses naming a
  Python exception class such as `json.JSONDecodeError` (P216)
* `print(..., file=)` and f-string format specs (`{x:.3f}`, `{x!r}`) use CPython's
  `print` and `format`

## Imports of Rust and C

See the README. Rejected signatures (generics, pointers other than `const char *`, structs,
variadic functions, `static` C functions) are listed in `explain`. Calling one gives P150.

## Rejected at compile time (diagnostic codes)

| Code | Meaning |
|---|---|
| P001 | Syntax error (from CPython's parser) |
| P100/P101 | Unsupported statement, expression, decorator or parameter form |
| P110 | Integer literal outside i64 |
| P003 | A `.rs` file that does not parse |
| P120 | Comprehensions and lambdas (closures are M6) |
| P145 | Circular import between natively compiled modules |
| P150 | Rust/C module has no callable function of that name (with the reason if its signature was skipped) |
| P140 | `from mod import NAME` where the native module has no such function or variable |
| P141/P142 | Rebinding a module-level function or import name; `nonlocal` (closures are phase 4) |
| P143/P144 | Undefined name; call before definition |
| P2xx | A CPython boundary, in `--mode native-only` only (see above). P220 is a warning: a local `.py` was imported through CPython |
| P301 (raise) | `raise` of a value that is not an exception |
| P301 | Operator or builtin applied to types where CPython raises `TypeError` |
| P302/P304/P305 | Value-dependent type in native-only mode (hybrid mode boxes these instead) |
| P306 | No return type could be inferred |
| P307 | Warning: a native value contradicts an annotation (it keeps its runtime type, like CPython) |
| P310/P311 | Specialization limit (16 per function) or inference did not converge |

Not yet supported in either mode: classes defined in natively compiled files (an imported
`.py` that defines a class falls back to CPython; a class in the entry file is rejected),
comprehensions, lambdas, closures, `*args`/`**kwargs` parameters, non-literal defaults,
and tuple unpacking.
