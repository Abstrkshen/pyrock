# Supported subset

## Compiled natively

These constructs become plain Rust with no Python objects involved:

* Top-level functions, including direct and mutual recursion. Parameters may be positional-only
  (`/`), positional-or-keyword or keyword-only (`*`), with defaults (any expression, evaluated
  once where the `def` runs, so a mutable default is shared as in CPython). Calls may pass keyword arguments; arguments are still
  evaluated in source order.
* `T | None`: a value that is `None` on some paths and a `T` on others is a native
  `Option<T>` (no boxing, also in native-only mode). Checks narrow it flow-sensitively: after
  `if x is None: continue` / `return` / `x = default`, inside `if x is not None:`, `if x:`,
  the right side of `x is not None and ...` and `a if x is not None else b`, reads of `x` are
  a plain `T`. Using it where a `T` is needed checks it
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
* f-strings, including format specs (`{x:>10}`, `{x:,.2f}`, `{x:#x}`, `{x:.1%}`, nested
  `{x:{w}}`) and `!r`/`!s`/`!a`; `format(x, spec)`; printf-style `"%s %5.2f" % (a, b)`;
  `"{} {name:>8}".format(...)`. The format-spec mini-language is implemented natively
  (fill/align/sign/`#`/`0`/width/grouping/precision and the `d x X o b c n e E f F g G %` types,
  correctly rounded). `%` with a dict (`%(name)s`) uses CPython.
* `*args` and `**kwargs` parameters (a native `tuple[T, ...]` and `dict[str, V]`), and
  `f(*xs)` / `f(**d)` calls of Python callables, `print(*xs)`, and `f(a, *rest)` into a
  native function's `*args`.
* Exceptions: `try`/`except`/`else`/`finally` (including `except (A, B) as e`, bare `except:`
  and bare `raise`), `raise E`, `raise E(msg)`, `raise e`, `assert cond[, msg]`, and builtin
  exception values such as `ValueError("x")`. `except` matches through CPython's builtin
  exception hierarchy (`except LookupError` catches `KeyError`); exceptions that come from
  CPython keep their original object, so `except json.JSONDecodeError` works through
  `isinstance`. `return`/`break`/`continue` inside `try` run `finally` first.
* `__name__` and `__file__`.
* Native collections: `list[T]` (shared `Rc<RefCell<Vec<T>>>`, so aliases see mutations),
  `dict[K, V]` (insertion-ordered; keys are int/str/bool/None/tuples of those), `set[T]`,
  fixed tuples (Rust tuples) and `tuple[T, ...]`, with indexing (negative indices,
  `IndexError`/`KeyError` as in CPython), slicing, `del`, `in`, `==`/`<`, `+`/`*`, `+=`, set
  algebra, `dict |`, live `keys()`/`values()`/`items()` views, and the methods `append`,
  `extend`, `insert`, `pop`, `remove`, `index`, `count`, `sort(reverse=)`, `reverse`, `copy`,
  `clear`, `get`, `setdefault`, `update`, `popitem`, `add`, `discard`, `union`, ...
* Element types are inferred from use: `xs = []` then `xs.append(1)` is a `list[int]`; mixed
  elements give `list[object]` (the list stays native, its elements are Python objects).
* Strings: indexing, slicing (code points; O(1) for ASCII), and `split`, `rsplit`, `join`,
  `strip`/`lstrip`/`rstrip`, `replace`, `find`/`rfind`/`index`/`count`, `startswith`/`endswith`,
  `upper`/`lower`/`title`/`capitalize`/`swapcase`, `is*` predicates, `center`/`ljust`/`rjust`/
  `zfill`, `partition`, `splitlines`, `removeprefix`/`removesuffix`, `expandtabs`.
* Builtins over iterables: `len`, `sorted(reverse=)`, `sum`, `min`/`max`, `any`, `all`, `list`,
  `tuple`, `set`, `dict`, `reversed`, `enumerate`, `zip`, `range(...)` as a value, `repr`,
  `ascii`, `ord`, `chr`, `divmod`, `round`, `hex`/`oct`/`bin`, and `isinstance(x, int)` on a
  native value (decided at compile time).
* Unpacking: `a, b = ...`, nested and starred targets (`first, *rest = xs`), swaps,
  `for k, v in d.items()`, `with ... as (a, b)`, with CPython's `ValueError` texts.
* Comprehensions: list, set and dict comprehensions, with nested `for` and `if` clauses;
  their variables do not leak. Generator expressions are lazy native generators.
* Escape rule: a native mutable container that reaches arbitrary Python code (an argument of
  a CPython call, stored into a Python object) is a Python object from creation, so Python
  can keep or mutate it with the aliasing CPython has. A short list of read-only consumers
  (`json.dumps`, `statistics.*`, ...) receives a copy instead.
* Classes: `__init__`, methods, instance attributes (types inferred program-wide), class
  attributes, `@property` (+ setter), `@staticmethod`, `@classmethod` (`cls(...)`), dunders
  (`__repr__`, `__str__`, `__eq__`/`__ne__`, `__lt__`/`__le__`/`__gt__`/`__ge__`, `__len__`,
  `__bool__`, `__hash__`, `__contains__`, `__getitem__`/`__setitem__`, `__iter__` returning a
  native iterable, `__add__`/`__radd__` and the other arithmetic dunders, `__neg__`, ...),
  `typing.NamedTuple` classes (fields, defaults, ordering, indexing, unpacking, `_asdict`),
  `Protocol`/`Generic[T]` bases, `TypeVar`, `NewType`, `TypedDict` (a plain dict),
  `@functools.cached_property`, enums (`Enum`, `IntEnum`, `StrEnum`, `auto()`: members, `C(value)`, `C["NAME"]`, iteration,
  `len`, `in`, `name`/`value`, CPython's repr/str), abstract base classes (`ABC`, `metaclass=ABCMeta`, `@abstractmethod`, with CPython's
  `TypeError` on instantiating one), `@dataclass` (fields, literal defaults, `field(default_factory=list|dict|set)`,
  `__post_init__`), single and multiple inheritance (mixins, diamonds; CPython's C3 MRO) with `super()`, `isinstance` with classes, and user
  exception classes (`class NotFound(KeyError)`, `except NotFound as e: e.key`).
  Instances are `Rc<RefCell<..>>` newtypes, so identity and shared mutation match Python.
  Values of different classes in one place (a list of shapes) form a closed union (a Rust
  enum) with per-variant method dispatch.
* `async def` / `await`: native coroutines run by asyncio (`asyncio.run(main())`,
  `gather`, `create_task`, `wait_for`, queues, ...). Awaiting a native coroutine runs it
  inline; awaiting a Python awaitable suspends the coroutine's own stack until the event
  loop has the result (P217 boundary). `async with` and `async for` work on Python objects
  and on native classes defining `__aenter__`/`__aexit__` or `__aiter__`/`__anext__`.
* Generators: `yield` and `yield from` in functions and methods, `send`/`throw`/`close`,
  `return value` (the value of `yield from`), `finally` when a generator is closed or
  abandoned. Async generators (`async def` with `yield`) with `async for`, `asend`, `athrow` and
  `aclose`. Generators still open at exit (in the main module or imported modules) are
  finalized as at CPython's shutdown.
  Async comprehensions (`[x async for x in g]`, `[await f(x) for x in xs]`) and
  async generator expressions. A call returns a native
  generator whose body runs on its own stack; `for` loops, `enumerate`, `zip` and `next`
  pull values lazily (infinite generators work), other builtins exhaust it. A generator
  passed to CPython is a Python iterator.
* Function values: lambdas, nested functions and closures (captured variables are shared
  cells, so late binding and `nonlocal` behave as in CPython; `lambda x, i=i:` defaults are
  evaluated when the function is created), top-level functions used as values, decorators
  (`@timed def f`), dispatch tables of different functions (a generated enum), `sorted`/
  `min`/`max` with `key=`, `list.sort(key=)`, `map` and `filter` (eager). Builtins passed as
  `key=` or to `map` (`key=len`) run natively.
* Native function values passed to CPython become Python callables (`re.sub(p, fn, s)`,
  `functools.reduce`, `@functools.lru_cache`, framework callbacks); CPython calls an
  all-`object` specialization, and exceptions cross both ways.
* Native instances passed to CPython are `native_object`s: Python can read fields, call
  methods (all-`object` specializations), assign scalar fields, print, compare and hash them,
  and gets the same Python object back each time (identity). Fields that hold mutable
  containers become Python objects so both sides see the same list.
* `from mod import Class` and `mod.Class(...)` for classes of natively compiled modules.
* A variable or field that holds an int on some paths and a float on others is a native
  `int | float` (each operation keeps exact int or float semantics).
* `math` functions and constants on native numbers (with CPython's domain/range errors);
  an `import math` used only that way needs no CPython.
* Container annotations on variables (`parts: list[str] = []`, `REG: dict[str, Agent] = {}`,
  `x: Optional[str] = None`) give a still-empty container its element types.
* Dicts with literal string keys (`{"x": 1.0, "name": "a"}`) are compiled as structs with
  one type per key (keys added later are optional fields). Reads, writes, `get`, `in`,
  `len`, `del` of added keys, printing, `==` and iterating keys are native; other dict uses
  turn that dict into a plain `dict[str, V]`.
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

| Behaviour | CPython | pyrock | Planned fix |
|---|---|---|---|
| int results outside i64 | promotes to bigint | `OverflowError` | M4: arbitrary precision / promotion |
| `int ** negative_int` with a non-literal exponent | float | `NotImplementedError` | M4: union return |
| negative float `**` fractional power | complex | `NotImplementedError` | out of scope |
| Traceback body | full frames | last line only | source-map-driven tracebacks |
| Set iteration / printing order | hash order | insertion order | — (string hashes are randomised in CPython anyway) |
| Dict views (`keys()`, ...) | support set operations | iteration, `len`, `in`, printing only | — |
| `RecursionError` threshold | counts every frame | counts recursive frames only | — |
| Python object returned into an `int`/`str`/`bool` annotation it doesn't match | annotation ignored | `TypeError` at the unbox | — (deliberate: unboxing must not change the value) |
| CPython threads during long native loops | interleave (GIL switch interval) | run only while compiled code waits (I/O, `sleep`, `join`, the event loop) | — (the GIL is held while native code runs) |
| Order of finalizing several imported modules that define functions, at exit | depends on the garbage collector (changes once `.pyc` files are cached) | one fixed order | — |

## Bridged through CPython (hybrid mode)

These are compiled, but they operate on `rt::py::Obj` values through the embedded interpreter.
Each site appears under "CPython boundaries" in `pyrock explain`, and each is a compile error
under `--mode native-only`.

* `import x` / `from x import y`, where no `x.py`, `x.rs` or `x.c` exists next to the file (P200)
* Attribute access, method calls and calls (with any arguments) on Python objects (P210)
* Builtins applied to Python objects, and the few builtins with no native version
  (`getattr`/`setattr`, `open`, `type`, `vars`, ...) (P210/P211)
* Containers that escape to Python, subscripts/slices of Python objects, attribute
  assignment on Python objects (P212)
* Operators and comparisons involving a Python object (P213), and `for x in <python iterable>` (P214)
* Variables or returns with no single native type (P215: boxed). `T | None`, `int | float`,
  unions of native classes and record dicts stay native
* `with` on a Python context manager, and `except` clauses naming a Python exception class
  such as `json.JSONDecodeError` (P216). `with` on a native class with `__enter__`/`__exit__`
  is native
* `await` of a Python awaitable, `async with` / `async for` on Python objects (P217)
* `print(..., file=)` uses CPython's `print`
* `bytes` literals (`b"..."`) are CPython `bytes` objects

## Imports of Rust and C

See `skills/pyrock/references/imports.md` for the type tables. Rust: scalars, `&str`/`String`,
`Vec<T>`/`&[T]`/`&mut [T]` (written back), `Vec<u8>` (bytes), `Option`, tuples, `HashMap`,
`impl Fn`/`&dyn Fn` callbacks, `pub struct` handles with `impl` methods and fields, and
`// pyrock: dep` crates. C: scalars, strings, pointer+length arrays (written back when not
`const`), `typedef struct` values, function pointers, `// pyrock: link` / `pkg-config`.
Rejected signatures (generics, unions, variadic functions, `static` C functions) are listed
in `explain`. Calling one gives P150. A struct handle passed to CPython is P231.

## Rejected at compile time (diagnostic codes)

| Code | Meaning |
|---|---|
| P001 | Syntax error (from CPython's parser) |
| P100 | Unsupported statement, expression, decorator or parameter form |
| P110 | Integer literal outside i64 |
| P002 | The frontend could not run (`python3` missing, AST protocol mismatch) |
| P003 | A `.rs` file that does not parse |
| P145 | Circular import between natively compiled modules |
| P150 | Rust/C module has no callable function of that name (with the reason if its signature was skipped) |
| P140 | `mod.NAME` / `from mod import NAME` where the native module has no such function, class or variable |
| P141 | Redefining a module-level function or class, or rebinding a function or import name |
| P142 | `global`/`nonlocal` of a name that is a function, class or import |
| P143/P144 | Undefined name; call before definition |
| P2xx | A CPython boundary, in `--mode native-only` only (see above). P220 is a warning: a local `.py` was imported through CPython |
| P231 | A Rust/C struct handle passed to CPython (both modes) |
| P217 | `await` of a Python awaitable, `async with`, `async for` (native-only: error) |
| P301 (raise) | `raise` of a value that is not an exception |
| P301 | Operator or builtin applied to types where CPython raises `TypeError` |
| P303 | Wrong arguments: count, unknown/duplicate keyword, wrong field type of a Rust/C struct |
| P302/P304/P305 | Value-dependent type in native-only mode (hybrid mode boxes these instead) |
| P306 | No return type could be inferred |
| P307 | Warning: a native value contradicts an annotation (it keeps its runtime type, like CPython) |
| P308 | Warning: an attribute that no code ever assigns is read (`AttributeError` at runtime) |
| P310/P311 | Specialization limit (16 per function; hybrid mode shares one `object` specialization instead) or inference did not converge |

Not yet supported in either mode (P100; in an imported `.py`, hybrid mode imports the
whole module through CPython instead, P220):

* classes with a CPython base class (other than `Enum`/`IntEnum`/`StrEnum`/`ABC`/`NamedTuple`), metaclasses other than `ABCMeta`,
  class decorators other than `@dataclass`, `@dataclass(unsafe_hash=...)`, adding class
  attributes after the class statement;
* `f(*xs)` where `xs` is a list (not a tuple of known length) filling fixed parameters,
  `**mapping` that would fill named parameters (into the callee's own `**kwargs` is fine),
  and `*`/`**` arguments to Rust/C functions;
* `match`, `type X = ...`, `except*`, `...` as a value, complex numbers, `del` of a
  function or import name, `:=` inside generator expressions and lambdas;
* `from x import *`;
* `globals()`, `locals()`, `vars()`/`dir()` without arguments, `eval`/`exec` without an
  explicit namespace (compiled code has no Python frame for them; `eval(src, ns)` works);
* arbitrary-precision integers (results beyond i64 raise `OverflowError`).
