# The Python subset pyfast compiles

The full, authoritative list is `docs/subset.md` in the repo. This is the working summary.

## Native (plain Rust, no Python objects)

* Functions: recursion, `/` and `*` parameter markers, literal defaults, keyword arguments,
  `*args` / `**kwargs` (fixed-shape tuples when the call site is known, so forwarding
  wrappers stay native).
* Module-level variables in functions (`RATE * x`), `global count`, `config.DEBUG`,
  `from config import LIMIT`: typed global cells, `NameError` if read before assignment.
* Lambdas, nested functions, closures (`nonlocal`), decorators and decorator factories,
  function values (`sorted(key=...)`, `map`, `filter`, dispatch tables, callbacks into
  Python libraries).
* Classes: methods, inferred instance attributes, class attributes, `@property`,
  `@staticmethod`/`@classmethod`, dunders, `@dataclass`, single inheritance with `super()`,
  user exception classes, and polymorphism over several classes (a generated enum).
  Instances passed to CPython are `native_object`s (fields, methods, identity).
* Native `list`/`dict`/`set`/`tuple` (element types inferred from use, aliasing preserved),
  slicing, comprehensions, unpacking (`a, *b = xs`, `for k, v in d.items()`), string
  methods, and builtins such as `sorted`, `sum`, `enumerate`, `zip`, `min`/`max`,
  `any`/`all`, `round`, `repr`.
* `T | None` is a native `Option<T>`: `def f(x, size=None): if size is None: size = 8`
  compiles without boxing.
* `try`/`except`/`else`/`finally`, `raise`, `assert`, following CPython's exception
  hierarchy. Exceptions from CPython keep their object (`except json.JSONDecodeError`).
* Generators: `yield`, `yield from`, generator methods. `for`, `enumerate`, `zip` and
  `next` pull lazily (infinite generators work).
* `async def` / `await`, `async with`, `async for`, run by asyncio (`asyncio.run`,
  `gather`, `create_task`, ...). Awaiting a native coroutine is a direct call; awaiting a
  Python awaitable suspends the coroutine's stack until the event loop has the result.
* f-strings with format specs, `%` formatting and `str.format`.
* Type annotations are optional. `int`/`float`/`str`/`bool` annotations unbox a Python
  object with a checked `TypeError`.

Type mapping: int → `i64`, float → `f64`, bool → `bool`, str → `rt::Str` (`Rc<str>`),
None → `()`, list → `rt::List<T>`, dict → `rt::Dict<K, V>`, class → `Rc<RefCell<..>>`
newtype, a value with no single type → `rt::py::Obj` (boxed, hybrid mode).

## Preserved Python semantics

* Floor division/modulo signs, true division (correctly rounded int/int), float special
  cases, exact int-vs-float comparison, `repr(float)` identical to CPython.
* bool is an int in arithmetic but prints `True`/`False`.
* Exceptions keep CPython's messages; an uncaught one exits with status 1 and prints
  `Kind: message` as the last stderr line.
* Possibly-unbound locals raise exactly where CPython would.
* print order is preserved between native output and output from CPython calls.

## Divergences

| Behaviour | CPython | pyfast |
|---|---|---|
| int beyond i64 | bigint | `OverflowError` |
| `int ** negative non-literal int` | float | `NotImplementedError` |
| negative float `**` fractional | complex | `NotImplementedError` |
| traceback | full frames | header + last line |
| generator expressions `(x for x in xs)` | lazy | evaluated eagerly into a list (generator *functions* are lazy) |
| set iteration order | hash order | insertion order |
| Python object into an `int`/`str`/`bool` annotation of another type | ignored | `TypeError` |

## Bridged through CPython (hybrid mode only)

Anything involving a CPython object: imported Python modules (stdlib, site-packages),
attribute access and method calls on their objects, `with` on Python context managers,
`await` of Python awaitables, and values with no single native type (boxed, P215). These
keep exact Python semantics. `pyfast explain` lists every such site; `--mode native-only`
turns them into errors.

## Not supported in any mode → how to rewrite

| Construct | Rewrite |
|---|---|
| Class with a CPython base class, a metaclass or a class decorator other than `@dataclass` | Keep it in an imported `.py` (the module falls back to CPython, P220) |
| Generator `send`/`throw`, `x = yield`, `return value` in a generator | Pass state through an object or a closure |
| Async generators (`async def` with `yield`) | A class with `__aiter__`/`__anext__`, or collect into a list |
| Non-literal default of a top-level function (`def f(x=[])`) | `def f(x=None): if x is None: x = []` |
| Relative imports, `import *`, imports inside functions | Absolute top-level imports |

## Making code fast

* Keep hot loops native: loops over ints, floats, strs and native collections, calling
  native or Rust/C functions. Check with `pyfast explain` that nothing in the loop is listed
  under "CPython boundaries".
* Keep one type per variable and per return; mixed types box the value (P215).
* Move numeric kernels over arrays into a `.rs` file: slices (`&[f64]`, `&mut [f64]`) take
  lists, `array.array` or numpy arrays directly.
* Use `--release` for benchmarks: `pyfast bench file.py --runs 5`.
