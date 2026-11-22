# How much of Python pyfast compiles

Two answers: which **language constructs** are compiled natively, and how much **real code**
gets through as it is, measured on CPython's standard library and three popular packages.
The exact rules are in [subset.md](subset.md). This page is about the size of what's
covered.

## In short

* **Language constructs:** 51 of the 55 statement and expression nodes in Python 3.12's
  `ast` module compile natively (table below). The 4 missing are `match`, the walrus
  operator `:=`, `type X = ...` and `except*`. Some forms of supported nodes are missing too:
  the `...` literal, complex numbers, and `*`/`**` unpacking inside list/dict literals. Several supported nodes have unsupported forms: `for ... else`,
  `del name`, `class` inside a function or `if`, relative imports, imports inside functions,
  non-literal default values, and classes with a CPython base class (`Enum`, `NamedTuple`,
  `Exception` subclasses are fine), multiple bases or a metaclass.
* **Values:** everything a program builds itself (numbers, strings, lists, dicts, sets,
  tuples, classes, closures, generators, coroutines) is native. Values from CPython imports
  (numpy arrays, sockets, `re` matches, ...) stay Python objects; in hybrid mode they work
  through CPython at reported boundaries, and native-only mode rejects them.
* **Real code, as it is:** 54% of the functions and methods in the standard library pass
  (49% of their lines). Third-party packages pass at 32–38%. Whole modules pass much less
  often (8% of stdlib modules), because one blocker in a module stops that whole module.
* **Code written for pyfast:** the 21 samples, 3 demos and 14 user samples all compile; the
  fully native ones need no CPython at all.

Library code is the hardest case: it is full of lazy imports, decorators from `abc` and
`functools`, `Enum`/`NamedTuple` classes and relative imports. Application code, especially
hot loops, uses far fewer of these. In hybrid mode none of this stops a program: a module
pyfast can't compile is imported through CPython instead (P220), and a CPython package is
always usable.

## Syntax nodes

All statement and expression nodes of Python 3.12's `ast` module.

| Node | Status | Notes |
|---|---|---|
| `FunctionDef` | native | parameters `/`, `*`, defaults (literal for top-level functions), `*args`, `**kwargs`, nested functions, decorators |
| `AsyncFunctionDef` | native | run by asyncio; also async generators |
| `ClassDef` | native, at module level | single inheritance; `@dataclass`; not inside a function or `if`, not with a CPython base class, several bases, `metaclass=` or other class decorators |
| `Return`, `Pass`, `Break`, `Continue`, `Expr` | native | |
| `Assign`, `AugAssign`, `AnnAssign` | native | including unpacking targets, attributes and subscripts |
| `Delete` | native for `del x[i]`, `del d[k]`, `del obj.attr` | `del name` is P100 |
| `For`, `AsyncFor` | native | `for ... else` is P100 |
| `While` | native | `while ... else` is P100 |
| `If` | native | |
| `With`, `AsyncWith` | native for native classes, bridged for Python objects | |
| `Raise` | native | including `raise X from Y` |
| `Try` | native | `except`, `else`, `finally`, `except (A, B) as e` |
| `TryStar` (`except*`) | not supported | |
| `Assert` | native | |
| `Import`, `ImportFrom` | at module level | `.py`/`.rs`/`.c` next to the file are compiled, everything else is a CPython import. Relative imports, `import *` and imports in functions are P100 |
| `Global`, `Nonlocal` | native | |
| `Match` | not supported | |
| `TypeAlias` (`type X = ...`) | not supported | |
| `BoolOp`, `BinOp`, `UnaryOp`, `Compare`, `IfExp` | native | `@` is bridged (numpy) |
| `NamedExpr` (`:=`) | not supported | |
| `Lambda` | native | |
| `Dict`, `Set`, `List`, `Tuple` | native | `{**d}` and `[*a, *b]` in literals are P100 |
| `ListComp`, `SetComp`, `DictComp`, `GeneratorExp` | native | generator expressions are lazy; async comprehensions too |
| `Await`, `Yield`, `YieldFrom` | native | |
| `Call` | native | keyword arguments, `f(*t)` with a tuple, `f(**d)` into Python callables |
| `JoinedStr`, `FormattedValue` | native | full format-spec mini-language |
| `Constant` | native for int (64-bit), float, str, bool, None | `bytes` are CPython objects; `...` and complex are P100; ints beyond 64 bits are P110 |
| `Attribute`, `Subscript`, `Slice`, `Starred`, `Name` | native | |

## Measured on real code

`python3 scripts/coverage.py [DIR]` copies each top-level `.py` file of a directory into a
scratch directory **on its own**, runs `pyfast check` on it, and maps every error to the
function or class it is in. A function or method counts as accepted when nothing inside it
and nothing on its class is rejected.

Results (pyfast 0.1.0, CPython 3.12.3, 2026-11-23):

| Corpus | Modules with no error | Functions + methods accepted |
|---|---|---|
| CPython standard library (top-level modules) | 14 / 167 (8%) | 3707 / 6823 (54.3%; 48.5% of lines) |
| `requests` | 2 / 19 | 102 / 267 (38.2%) |
| `huggingface_hub` | 0 / 35 | 329 / 877 (37.5%) |
| `langchain_core` | 4 / 18 | 35 / 108 (32.4%) |

What the measurement means:

* `check` infers types for code that the module itself reaches. A library module mostly
  defines functions that nothing in the module calls, so for those the measurement checks
  that every construct is supported, not that every type works out. Read it as an upper
  bound for "compiles natively once called" and a measure of syntax coverage.
* Module-level errors (an import inside `try:` at the top, a relative import) don't count
  against any one function, although they stop the whole module. Hence the gap between the
  two columns.
* Each file is checked alone, so its imports are CPython imports. A blocker in a sibling
  module doesn't count twice.

The most common blockers in the standard library, by count (undefined-name errors are left
out because most of them follow from another blocker, such as an import inside a function):

| Count | Blocker | Typical source |
|---|---|---|
| 452 | Import inside a function or a `try`/`if` block | lazy imports, `try: import _speedup` fallbacks |
| 233 | Non-literal default value (`def f(x=_sentinel)`, `sep=os.sep`) | sentinels, module constants |
| 152 | Method decorator other than `@property`/`@staticmethod`/`@classmethod` | `@abstractmethod` (107), `@functools.cached_property`, `@lru_cache`, `@contextmanager` |
| 110 | Computed class-body assignment | `__add__, __radd__ = _operator_fallbacks(...)`, dispatch tables |
| 101 | Base class that isn't a native class | `Enum`, `NamedTuple`, `dict`, `threading.Thread`, ABCs |
| 54 | `del name` | cleaning up module namespaces |
| 41 | Class keywords | `metaclass=ABCMeta` |
| 40 | `class` inside a function or block | |
| 39 | Multiple inheritance | mixins |
| 38 / 4 | `for ... else` / `while ... else` | |
| 35 | Walrus `:=` | |
| 22 | Class decorators | `@functools.total_ordering`, `@dataclass(frozen=True)` options |
| 20 | `[*a, *b]` / `{*a}` | |
| 11 | `from x import *` | |

In third-party packages, relative imports (`from . import utils`) lead, followed by imports
inside functions and method decorators.

Removing the first three blockers (lazy imports, non-literal defaults, `abc`/`functools`
method decorators) would accept most of the remaining stdlib functions. They are the next
items in [roadmap.md](roadmap.md) phase 8.

## What this doesn't cover

* **Semantics beyond syntax:** integers are 64-bit (`OverflowError` instead of a big
  integer), tracebacks show only the last line, and the other differences are listed in
  [subset.md](subset.md#known-divergences-documented-tested-where-possible).
* **The dynamic features** are never assumed natively. `getattr`/`setattr` with computed
  names work on Python objects through CPython. `eval` and `exec` with an explicit
  namespace (`eval(src, {"x": 1})`) run in CPython. Without one, and `globals()`,
  `locals()`, `vars()` and `dir()` without arguments, are rejected (P100): compiled code
  has no Python frame for them to read. A module that uses them is imported through
  CPython as a whole in hybrid mode (P220), so it behaves exactly as in CPython. Rebinding module-level functions is rejected (P141).
