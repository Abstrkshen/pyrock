# The Python subset pyfast compiles

## Native (pure Rust, no Python objects)

* Top-level `def`, including recursion. `/` and `*` parameter markers, literal defaults
  (`def f(x, n=None, *, sep=",")`) and keyword arguments at the call are native.
* Module-level variables in functions (`RATE * x`), `global count`, `config.DEBUG`,
  `from config import LIMIT`: typed global cells, `NameError` if read before assignment.
* `T | None` is a native `Option<T>`: `def f(x, size=None): if size is None: size = 8`
  compiles without boxing. Arithmetic on a `None` raises CPython's `TypeError`.
* Locals: one concrete type per specialization.
* Literals: `int` (i64), `float`, `str`, `bool`, `None`.
* Operators: `+ - * / // % ** & | ^ << >> ~`, unary `- + not`, comparisons, including chains
  (`a < b <= c`), `in` / `not in` on str, `is` / `is not` with None.
* `and`/`or` (returning the deciding operand), `x if c else y`.
* `if/elif/else`, `while`, `for i in range(a[, b[, c]])`, `break`, `continue`, `pass`,
  `return`, augmented assignment, chained assignment (`a = b = 0`).
* Builtins: `print(*args)`, `abs`, `int`, `float`, `str`, `bool`, `len(str)`, and
  `min`/`max` with 2+ arguments of compatible types.
* f-strings: `{expr}` is native. Format specs (`{x:.3f}`, `{x!r}`) use CPython's `format`.
* `if __name__ == "__main__":` works, and `__file__`.
* `try`/`except`/`else`/`finally`, `except (A, B) as e`, bare `except:` and `raise`,
  `raise E(msg)`, `assert`. `except` follows CPython's exception hierarchy; exceptions from
  CPython code keep their object (`except json.JSONDecodeError` works, as a P216 boundary).
* Type annotations on parameters, returns and `x: T = v`. `int`, `float`, `str`, `bool`,
  `None`, `object`/`Any` and string forward refs are understood. Others (`list[int]`,
  `Optional`) are accepted and ignored.

Type mapping: int → `i64`, float → `f64`, bool → `bool`, str → `rt::Str` (`Rc<str>`),
None → `()`, other → `rt::py::Obj`.

## Preserved Python semantics

* Floor division/modulo signs, true division (correctly rounded int/int), float special
  cases, exact int-vs-float comparison.
* `repr(float)` identical to CPython (`1e+16`, `1e-05`, `-0.0`, `inf`, `nan`).
* bool is an int in arithmetic but prints `True`/`False`.
* Exceptions: `ZeroDivisionError`, `ValueError`, `OverflowError`, `TypeError`,
  `UnboundLocalError`, `NameError`, `RecursionError` (limit 1000, counted only in recursive
  functions), plus any exception from CPython with its qualified name.
* Possibly-unbound locals raise exactly where CPython would.
* print order is preserved between native output and output from CPython calls.

## Divergences

| Behaviour | CPython | pyfast |
|---|---|---|
| int beyond i64 | bigint | `OverflowError` |
| `int ** negative non-literal int` | float | `NotImplementedError` |
| negative float `**` fractional | complex | `NotImplementedError` |
| traceback | full frames | header + last line |
| Python object into an `int`/`str`/`bool` annotation of another type | ignored | `TypeError` |

## Bridged through CPython (hybrid mode only)

Lists, dicts, tuples, sets, subscripts and slices, item and attribute assignment, method
calls and attributes, keyword arguments, calls of Python objects, `for x in <iterable>` other
than range, builtins without a native version (`sorted`, `sum`, `round`, `isinstance`,
`repr`, `enumerate`, `zip`, `range` used as a value, ...), `with` (context managers),
`print(file=)`, and boxed
values (mixed types). These keep exact Python semantics, including aliasing.

## Not supported in any mode → how to rewrite

| Construct | Rewrite |
|---|---|
| `class` in the entry file | Move it into a separate `.py` module (it falls back to CPython, with P220), or use functions plus separate values |
| `[f(x) for x in xs]`, generators | `out = []` + `for` + `out.append(...)` |
| `lambda`, nested `def`, closures | A top-level function |
| `*args`, `**kwargs` parameters | Pass a list or explicit parameters |
| Non-literal default (`def f(x=[])`) | `def f(x=None): if x is None: x = []` |
| `a, b = b, a` / `for k, v in d.items()` | `t = a; a = b; b = t` / index the tuple: `kv[0]`, `kv[1]` |
| `nonlocal` | Return the new value (closures are phase 4) |
| decorators | Call the wrapper explicitly |

## Making code fast

* Keep hot loops native: `range` loops over ints, floats and strs, calling native or Rust/C
  functions. Check with `pyfast explain` that nothing in the loop is listed under
  "CPython boundaries".
* Move numeric kernels that need arrays into a `.rs` file (pass scalars, or do the whole loop
  in Rust).
* Use `--release` for benchmarks: `pyfast bench file.py --runs 5`.
