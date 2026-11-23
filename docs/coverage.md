# How much of Python pyfast compiles

Two answers: which **language constructs** are compiled natively, and how much **real code**
gets through as it is, measured on CPython's standard library and three popular packages.
The exact rules are in [subset.md](subset.md). This page is about the size of what's
covered.

## In short

* **Language constructs:** 52 of the 55 statement and expression nodes in Python 3.12's
  `ast` module compile natively (table below). The 3 missing are `match`, `type X = ...`
  and `except*`. Some forms of supported nodes are missing too: `class` inside a function
  or `if`, `import *`, complex numbers, `...` as a value, and classes with a CPython base
  class (`NamedTuple`, `dict`, ...; `Exception`, `Enum` and `ABC` subclasses are fine),
  multiple bases or another metaclass.
* **Values:** everything a program builds itself (numbers, strings, lists, dicts, sets,
  tuples, classes, closures, generators, coroutines) is native. Values from CPython imports
  (numpy arrays, sockets, `re` matches, ...) stay Python objects; in hybrid mode they work
  through CPython at reported boundaries, and native-only mode rejects them.
* **Real code, as it is:** 62% of the functions and methods in the standard library pass,
  and 68% / 73% / 41% of those in the `requests` / `huggingface_hub` / `langchain_core`
  packages. Whole modules pass much less often (41 of 167 stdlib modules), because one
  blocker in a module stops that whole module.
* **Code written for pyfast:** the 21 samples, 3 demos and 14 user samples all compile; the
  fully native ones need no CPython at all.

Library code is the hardest case: it is full of decorators from `abc` and `functools`,
`Enum`/`NamedTuple` classes, mixins and relative imports. Application code, especially
hot loops, uses far fewer of these. In hybrid mode none of this stops a program: a module
pyfast can't compile is imported through CPython instead (P220), and a CPython package is
always usable.

## Syntax nodes

All statement and expression nodes of Python 3.12's `ast` module.

| Node | Status | Notes |
|---|---|---|
| `FunctionDef` | native | parameters `/`, `*`, defaults (any expression, evaluated once at `def` time), `*args`, `**kwargs`, nested functions, functions defined in `if`/`try` blocks, decorators |
| `AsyncFunctionDef` | native | run by asyncio; also async generators |
| `ClassDef` | native, at module level | single inheritance; `@dataclass` (with options); `Enum`/`IntEnum`/`StrEnum`; `ABC`/`ABCMeta` with `@abstractmethod`; not inside a function or `if`, not with a CPython base class, several bases, another metaclass or other class decorators |
| `Return`, `Pass`, `Break`, `Continue`, `Expr` | native | |
| `Assign`, `AugAssign`, `AnnAssign` | native | including unpacking targets, attributes and subscripts |
| `Delete` | native | `del x[i]`, `del d[k]`, `del obj.attr`, `del name` (unbound afterwards) |
| `For`, `AsyncFor` | native | including `for ... else` |
| `While` | native | including `while ... else` |
| `If` | native | |
| `With`, `AsyncWith` | native for native classes, bridged for Python objects | |
| `Raise` | native | including `raise X from Y` |
| `Try` | native | `except`, `else`, `finally`, `except (A, B) as e` |
| `TryStar` (`except*`) | not supported | |
| `Assert` | native | |
| `Import`, `ImportFrom` | native | `.py`/`.rs`/`.c` files and packages next to the program are compiled, including relative imports; everything else is a CPython import. Also inside functions and `try`/`if` blocks. `import *` is P100 |
| `Global`, `Nonlocal` | native | |
| `Match` | not supported | |
| `TypeAlias` (`type X = ...`) | not supported | |
| `BoolOp`, `BinOp`, `UnaryOp`, `Compare`, `IfExp` | native | `@` is bridged (numpy) |
| `NamedExpr` (`:=`) | native | in conditions, loops and list/set/dict comprehensions (binds the enclosing scope); not in generator expressions or lambdas |
| `Lambda` | native | |
| `Dict`, `Set`, `List`, `Tuple` | native | including `[*a, *b]`, `(*a, x)`, `{*a}`, `{**d, "k": v}` |
| `ListComp`, `SetComp`, `DictComp`, `GeneratorExp` | native | generator expressions are lazy; async comprehensions too |
| `Await`, `Yield`, `YieldFrom` | native | |
| `Call` | native | keyword arguments, `f(*t)` with a tuple, `f(**d)` into Python callables |
| `JoinedStr`, `FormattedValue` | native | full format-spec mini-language |
| `Constant` | native for int (64-bit), float, str, bool, None | `bytes` are CPython objects; `...` as a statement (a placeholder body) is `pass`, as a value it is P100; complex is P100; ints beyond 64 bits are P110 |
| `Attribute`, `Subscript`, `Slice`, `Starred`, `Name` | native | |

## Measured on real code

`python3 scripts/coverage.py [DIR]` copies each top-level `.py` file of a directory into a
scratch directory **on its own**, runs `pyfast check` on it, and maps every error to the
function or class it is in. A function or method counts as accepted when nothing inside it
and nothing on its class is rejected. When DIR is a package (it has `__init__.py`), every
module of the package is checked in place instead, imported by a driver file, so relative
imports resolve; CPython boundaries don't count as errors there.

Results (pyfast 0.1.0, CPython 3.12.3, 2026-11-23):

| Corpus | Modules with no error | Functions + methods accepted |
|---|---|---|
| CPython standard library (top-level modules) | 41 / 167 (25%) | 4260 / 6823 (62.4%; 62.2% of lines) |
| `requests` (all modules, in place) | 9 / 19 | 182 / 267 (68.2%; 79.7% of lines) |
| `huggingface_hub` (all modules, in place) | 53 / 182 | 1538 / 2103 (73.1%) |
| `langchain_core` (all modules, in place) | 45 / 188 | 969 / 2339 (41.4%) |

Before phase 8 started (same day): stdlib 14 modules and 54.3% of functions. The packages
were then measured one top-level file at a time with relative imports rejected (`requests`
38.2%, `huggingface_hub` 37.5%, `langchain_core` 32.4%), so their old and new numbers aren't
comparable; package support is what lets them be measured in place at all. Imports inside
functions and blocks, non-literal defaults, functions defined in module-level blocks,
`for`/`while ... else`, `:=`, `del name` and `*`/`**` in literals account for the stdlib
difference.

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

The most common blockers left in the standard library, by count (undefined-name errors are
left out because most of them follow from another blocker):

| Count | Blocker | Typical source |
|---|---|---|
| 110 | Computed class-body assignment | `__add__, __radd__ = _operator_fallbacks(...)`, dispatch tables |
| 96 | Base class that isn't a native class | `NamedTuple`, `dict`, `threading.Thread`, `collections.abc` ABCs |
| 44 | `class` inside a function or block, `match` | |
| 42 | Method decorator other than `@property`/`@staticmethod`/`@classmethod`/`@abstractmethod` | `@functools.cached_property`, `@lru_cache`, `@contextmanager` |
| 39 | Multiple inheritance | mixins |
| 33 | `eval`/`exec`/`globals()`/`locals()` without a namespace | |
| 29 | Class keywords | a metaclass other than `ABCMeta` |
| 27 | Other statements in a class body | `if` / `for` in a class body |
| 25 | A module-level name bound both by `def`/`import` and by assignment | `try: from _c import f` / `except: f = None` |
| 24 | A method or class attribute used as a value | `Class.method` passed around |
| 22 | Class decorators | `@functools.total_ordering` |
| 22 | Adding class attributes after the class statement | `Cls.x = ...` at module level |
| 20 | `del` of a function or import name | module cleanup: `del _f`, `del re` |

In third-party packages, method decorators lead (`langchain_core`: 542, mostly its own
decorators such as `@deprecated` and `@override`), then CPython base classes
(`pydantic.BaseModel`, `typing.Protocol`, `TypedDict`) and `f(*args)` calls into native
functions with fixed parameters (121).

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
