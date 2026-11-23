# Roadmap: Python for real apps

Goal: compile enough of the Python that **backend, AI and GUI apps** actually use, and make
existing **C and Rust projects** easy to use from it. Not all of Python: anything dynamic
keeps running in the embedded CPython, at a reported boundary.

For a summary of where things stand now (features, measured speed, known differences), see
[status.md](status.md).

This file tracks progress. Each step has an acceptance gate, and the log at the bottom
records what changed and how it was verified. `Project.md` §13 milestone M6 ("collections,
classes, closures, callbacks") is split into the phases below.

Status: `[ ]` not started · `[~]` in progress · `[x]` done (gate met).

## Phase 0: Basics

- [x] git repository with an initial commit
- [x] Relative `--python` path works (regression test `relative_python_path`)
- [x] Generated Rust compiles without warnings (`dead_code` was leaking into `pyrock run` output)
- [x] `scripts/readme_check.sh`: runs every sample README's commands verbatim
- [x] CI workflow: `cargo test`, `samples/run_all.sh`, `scripts/readme_check.sh`

## Phase 1: Type system and everyday syntax

- [x] **Parametric types.** `Ty` becomes an interned, `Copy` handle that can express
      `list[T]`, `dict[K, V]`, `tuple[...]`, `set[T]`, class instances, functions and
      `Optional[T]`. Gate: behaviour-preserving, every golden snapshot unchanged.
- [x] `try` / `except` / `else` / `finally`, `raise`, `assert`, exception hierarchy for builtin types
- [x] `with` (CPython context managers; native ones once classes land), `__file__`
- [x] Default parameter values, keyword arguments to native functions, `print(sep=, end=, flush=)`
- [x] `Optional[T]` as a native `Option<T>`; `x is None` folded per specialization
- [x] Module-level variables readable inside functions, `global`, `mod.NAME`, `from mod import NAME`
- Tuple unpacking and `*args`/`**kwargs` moved to phase 2 (they need native tuples).

## Phase 2: Native collections

- [x] `list[T]` as a shared handle (`Rc<RefCell<Vec<T>>>`), so aliasing matches CPython
- [x] `dict[K, V]` (insertion-ordered, live views), `tuple` (fixed and `tuple[T, ...]`), `set[T]`
- [x] Methods, `len`, `in`, indexing, slicing, iteration, `del`, operators
- [x] Builtins: `enumerate`, `zip`, `sorted`, `sum`, `reversed`, `list()`, `dict()`, `min`/`max`, `any`/`all`, `round`, `divmod`, ...
- [x] Native string indexing, slicing and methods
- [x] List / dict / set comprehensions and generator expressions (inlined as loops)
- [x] Tuple unpacking (`a, b = f()`, `for k, v in d.items()`, starred targets, swaps)
- [x] Escape rule: a collection that reaches Python is a Python object from creation
- [x] `*args` / `**kwargs` parameters, star-calls of Python callables and `print(*xs)`
- [x] Native f-string format specs, `%` formatting and `str.format`

## Phase 3: Classes

- [x] Classes with `__init__`, fields inferred from `self.x = ...`, methods, class attributes,
      `@property`/setters, `@staticmethod`, `@classmethod`, operator and builtin dunders
- [x] `@dataclass` (fields, literal defaults, `field(default_factory=list/dict/set)`, `__post_init__`, repr, ==)
- [x] Single inheritance, method overriding, `super()`, `isinstance` with native classes
- [x] User-defined exception classes (raise, `except MyError as e` with attributes, hierarchy matching)
- [x] Polymorphism: instances of different classes in one place form a closed union (a Rust enum) with per-variant dispatch
- [x] Dynamic classes (metaclasses, Python base classes) in an imported module fall back to CPython with the module (P220); in the entry file they are rejected with a diagnostic

## Phase 4: Closures and callbacks

- [x] Lambdas, nested functions, captured variables, `nonlocal`, defaults evaluated at creation
- [x] Functions as values, passing native functions to native functions, `key=` functions, `map`/`filter`, decorators, dispatch tables
- [x] Native functions callable from Python (callbacks such as `re.sub(p, fn, s)`, `functools.lru_cache`, LangChain `RunnableLambda`)
- [x] Native class instances visible to Python (attributes, methods, repr, ==, identity), e.g. `json.dumps(objs, default=...)`, `getattr`, `sorted` by Python
- [x] `f(*args, **kwargs)` forwarding into a native function's fixed parameters (wrappers, decorator factories, stacked decorators)

## Phase 5: Rich Rust and C interop

- [x] Lists and slices (`Vec<T>`, `&[T]`, `T *` + length) across the boundary
- [x] Buffers (`bytes`, buffer protocol, numpy arrays)
- [x] Structs ↔ classes; Rust `impl` methods; opaque handles
- [x] Callbacks (function pointers, `Fn` arguments)
- [x] Cargo dependencies for `.rs` files; library / pkg-config flags for `.c` files

## Phase 6: async

- [x] `async def` / `await` bridged to asyncio (`async with`, `async for`)
- [x] Generators (`yield`, `yield from`) on the same stackful coroutines
- [x] Lazy generator expressions
- [x] Generator `send`/`throw`/`close`, `return value`, `finally` on abandoned generators
- [x] Async generators
- [x] `asend` / `athrow` on async generators
- [x] Async comprehensions and async generator expressions
- [x] Finalization of the main module at exit (open generators, Python objects)
- [x] Finalization of imported native modules at exit

## Phase 7: speed of mixed code

- [x] Cheaper CPython bridge (attached for the whole run, interned constants, no per-call flush)
- [x] Native functions and objects callable from CPython threads
- [x] Per-key types for dicts with fixed string keys (record dicts → structs)
- [x] Native `math` module
- [x] A native `int | float` value (a variable that holds both now boxes)

## Throughout

- [x] Install without depending on the repo path (runtime crate shipped with the binary)
- [x] Demo apps compiled and checked in CI: backend (JSON API handlers), AI (tokenizer +
      C/Rust inference glue), GUI (raylib; through the `raylib` Python package, see the log)

## Phase 8: what is still open

From a review of `Project.md` against the implementation (2026-11-23) and the coverage
measurement in [coverage.md](coverage.md). Not started unless marked.

Problems found while building `examples/ollama-agents` (its `FINDINGS.md`; repros in
its `repro/` folder), which block ordinary app code:

- [x] F-01: `x is None` checks don't narrow `str | None` for later uses (P301/P307)
- [x] F-02: an empty module-level dict filled later (`AGENTS = {}`) gives unknown value
      types; `AGENTS: dict[str, Agent] = {}` is ignored (P215)
- [x] F-03: a list attribute appended in one method and read in another has an unresolved
      element type (P215)
- [x] F-05: annotations on local variables (`parts: list[str] = []`) are ignored

Language coverage (ordered by how much code each one blocks in the standard library):

- [x] Imports inside functions and in `try`/`if` blocks (452 stdlib sites)
- [x] Non-literal default values of top-level functions, evaluated once at definition (233)
- [x] Functions defined inside module-level `if`/`try` blocks
- [x] `@abstractmethod` with `ABC` / `metaclass=ABCMeta`
- [x] `@dataclass(...)` options (frozen, order, eq, repr, kw_only, slots)
- [x] `@functools.cached_property`
- [ ] `@lru_cache` on methods, `@contextmanager` on methods
- [x] `del name`, `for`/`while ... else`, `:=`, `[*a, *b]`, `{**d}`, `...` placeholders
- [ ] Computed class-body assignments, `del` of a function or import name, `class` inside a function or block
- [x] `Enum`, `IntEnum`, `StrEnum`, `auto()`
- [x] `typing.NamedTuple` classes
- [x] Multiple inheritance (mixins, diamonds) with CPython's C3 MRO
- [x] `Protocol` / `Generic[T]` bases, `TypeVar`, `NewType`, `TypedDict`
- [ ] `collections.namedtuple`
- [ ] Clearer native-only diagnostics for values of several types: a field (reported as P306) and `dict | dict` with different value types (reported as a CPython TypeError)
- [x] `f(**kwargs)` forwarded into a native function's own `**kwargs`
- [ ] `f(*xs)` with a list filling fixed parameters; `**mapping` filling named parameters
- [x] Packages next to the program, relative imports, re-exported names
- [ ] `match`
- [ ] Arbitrary-precision integers (`Project.md` §6 and M4; today results beyond i64 raise `OverflowError`)

`Project.md` items not implemented:

- [ ] `strict` mode (§4): reject every dynamic fallback, combinable with hybrid/native-only
- [ ] Release the GIL during long native computations (§8). Today it is held while native
      code runs, so CPython threads only run while compiled code waits
- [ ] Closed unions of scalar types such as `int | str` (§6); today they are boxed
- [ ] A typed block/SSA IR with a verifier (§9). The compiler specializes the HIR directly
      and relies on rustc to check the emitted code
- [ ] Tracebacks with every frame, through the source map (only the last line today)
- [ ] Build checks on macOS (§12; CI runs on Linux only)
- [ ] Benchmarks of compile time, startup latency, peak RSS, binary size and bridge
      overhead (§12; only wall time is measured)
- [ ] Optional C backend (M6)

Known bugs:

- [x] `isinstance` doesn't narrow a union of classes: P308 false warnings, `range(len(v.xs))` rejected (P301), and in a larger program invalid Rust that rustc rejects (fixed; `tests/differential/isinstance_narrowing.py`)
- [ ] `zip` over a class's list attribute and a parameter collapses inference for the whole program to `object` (`tests/known_bugs/zip_collapses_inference.py`)
- [ ] Overridden methods across a class hierarchy collapse inference to `object` (`tests/known_bugs/method_overrides_collapse.py`)
- [ ] Generator expressions cost about 6 µs per creation natively (slower than CPython in `" ".join(... for ...)` hot loops)

- [x] `globals()` panicked at runtime; `eval`, `exec` and `locals()` raised
      `SystemError: frame does not exist` (compiled code has no Python frame)
- [x] `examples/raylib-breakout`: the camera offset reached `draw_sky` as `object`
      although every call site passes a native number, so part of the per-frame drawing was
      bridged (stale specializations counted towards the limit)

## Log

### Phase 0 (2026-11-22)

* `git init`, `.gitignore` (target, `.pyrock/`, `__pycache__`, `.ignore/`, built `.so`).
* `--python` relative paths are made absolute (`std::path::absolute`, not `canonicalize`, so
  a venv interpreter keeps its site-packages). Test: `relative_python_path` builds
  `tests/cli/fallback_imports` from `/` with a relative interpreter path.
* Generated code now has `#![allow(dead_code)]`. Unused import slots (from local `.py`
  modules that fall back to CPython) printed cargo warnings above program output. The same
  test fails without the fix. Goldens re-blessed; the only change is that allow line.
* `scripts/readme_check.sh` runs README commands verbatim against the fresh debug build:
  11/11 sample projects pass. `samples/run_all.sh`: 21/21 pass.

### Phase 1.1: parametric types (2026-11-22)

* `Ty` gains `Ty::C(TyId)`: an index into a process-wide interner of `Comp` values
  (`List`, `Dict`, `Set`, `Tuple`, `Class`, `Func`, `Opt`). `Ty` stays `Copy`/`Hash`/`Ord`,
  so `SpecKey` and every existing `match` keep working. Interned names are leaked once per
  distinct type so `py_name`/`rust`/`mangle` still return `&'static str`.
* Constructors and views: `Ty::list(t)`, `Ty::dict(k, v)`, `Ty::tuple(..)`, `Ty::opt(t)`,
  `t.list_elem()`, `t.dict_kv()`, `t.is_mutable()`, ...
* Codegen routes composites through generic runtime entry points (`rt::truthy`,
  `rt::conv::str_of`, `rt::py::to_py`, `rt::ops::*`) that phase 2 implements.
* Gate met: `cargo test --workspace` passes with every golden snapshot unchanged.

### Phase 1.2: exceptions and `with` (2026-11-22)

* HIR: `StmtKind::Try { body, handlers, orelse, finalbody }`, `Raise`, `With`, and
  `ExprKind::NewExc` for `ValueError(...)`-style calls (0 or 1 argument; more go through
  CPython). `assert` lowers to `if not c: raise AssertionError(msg)`.
* Codegen: the `try` body runs in an immediately-called closure returning
  `PyResult<rt::Flow<R>>`, so `?` inside it is intercepted. `return`/`break`/`continue` that
  leave the closure become `Flow` values and are re-issued after `finally`. Functions with
  closures initialise their locals (closures cannot capture uninitialised bindings).
* Runtime: `rt::Exc` keeps the original Python exception (`origin`) when it came from
  CPython, so matching uses `isinstance` and re-raising preserves identity. Native matching
  uses a table of CPython's builtin hierarchy (`rt::exc::HIERARCHY`). New traits:
  `rt::conv::PyFmt` (str/repr), `rt::Truthy`, `rt::py::ToPy`.
* `with` uses CPython's context-manager protocol (`__enter__`/`__exit__`, suppression
  honoured) and is reported as boundary P216.
* `__file__` is supported (path as given for the entry file, absolute for imports).
* Tests: `tests/differential/exceptions.py` (hierarchy matching, tuples of types, `as e`,
  else/finally ordering, break/continue through finally, bare re-raise, JSON errors from
  CPython, `assert`, an uncaught exception at the end). The fallback fixture now uses a
  metaclass, because its `try` compiles natively now.
* Effect: `examples/rust-http/client.py` (uses `try`/`with`) now compiles natively
  instead of falling back. `samples/run_all.sh` 21/21, `scripts/readme_check.sh` 11/11.

### Phase 1.3: Optional, defaults and keyword arguments (2026-11-22)

* `T | None` is native: joining `None` with `T` gives `Ty::opt(T)` (`Option<T>`) instead of
  boxing (hybrid) or P304/P305 (native-only); functions that fall off the end return
  `T | None`. Operators, ordering comparisons, numeric builtins, `range` bounds and Rust/C
  arguments unwrap it with `rt::unopt`, raising CPython's exact `TypeError` text on `None`.
  `==`/`is` with `None`, `print`, truthiness and boxing work on the `Option` directly.
* Per-specialization constant folding: `x is None` / `is not None` on a local whose type is
  exactly `None` or never `None` (combined with `not`/`and`/`or`) decides `if`, `while`,
  `and`/`or` and `a if c else b` at compile time, so dead branches are neither type-checked
  nor emitted. Only side-effect-free tests fold.
* Parameters whose local type is wider than the argument type (a `None` default later
  assigned an int) are rebound with a coercion at function entry. This also fixes a latent
  bug for boxed parameters.
* Signatures: `/`, `*`, keyword-only parameters and literal defaults. Calls bind keyword
  arguments and defaults at compile time (CPython's TypeError wording for unexpected,
  duplicate, missing and surplus arguments, as P303). When keyword arguments are out of
  parameter order they are evaluated into temporaries in source order.
* `print(..., sep=, end=, flush=)` is native (`rt::io::print_kw`); only `file=` still uses
  CPython.
* Tests: `tests/differential/optional.py`, `tests/differential/call_args.py` (the latter
  passes `--mode native-only`), `tests/diagnostics/call_errors.py`. Golden change: `maybe()`
  in `annotations.py` is now native `-> int | None` instead of a boxed object.

### Phase 1.4: module globals (2026-11-22)

* A pre-scan finds module variables that functions read or declare `global`; every variable
  of an imported native module is one too. They become typed global cells
  (`thread_local! RefCell<Option<T>>`, read with `rt::gget` which raises `NameError` while
  unbound, written with `rt::gset`). HIR: `ExprKind::Global`, `StmtKind::AssignGlobal`.
* The type of a global joins every assignment in the program (module bodies, `global`
  writes in functions, `mod.NAME = v` from other modules) in `Program::globals`, inside the
  same fixed point as specialization.
* `from mod import NAME` binds a global of the importer, assigned once at the import
  statement (CPython copies the value at import time; a later `mod.NAME = v` does not
  change it).
* Loop, `with` and `except ... as` targets that are globals bind a synthetic local that is
  copied into the cell at the start of the body.
* Tests: `tests/differential/globals.py` (+ `_settings.py`): constants in functions, a
  `global` counter, cross-module reads/writes, from-import copy semantics, a loop variable
  read by a function, `NameError` before assignment. Passes `--mode native-only`.
  `tests/diagnostics/rebinding.py` now covers P141 (P140 for reads is gone).

### Phase 2.1: native collections (2026-11-22)

* Runtime (`crates/runtime/src/coll.rs`): `List<T>` (shared `Rc<RefCell<Vec<T>>>`),
  `Dict<K, V>` (insertion-ordered table with tombstones; iteration raises CPython's
  `RuntimeError` if the size changes), `Set<T>`, `Tup<T>` (`tuple[T, ...]`), `Tuple0`, live
  dict views `Keys`/`Values`/`Items`, and `Never` (element type of a container that never
  receives one). Traits `PyEq`/`PyOrd` give Python equality and ordering; `PyFmt` prints
  like CPython. Strings gained native indexing, slicing and ~40 methods (`string.rs`).
* Types: `Comp::{List, Dict, Set, Tuple, TupleVar, View, Never}`. `native::ty_join` joins
  element types (`list[int] ⊔ list[str] = list[object]`), so containers are never copied
  because their element types disagree.
* Inference widens *places*: `xs.append(1)` widens `xs`; stores through `xs[i] = v`,
  `d[k] = v`, `d.setdefault(k, []).append(x)` widen the container; assignments of mutable
  values join both ways (aliases share one Rust type); callees that widen a parameter widen
  the caller's argument; callers that need a wider return type widen the callee's return
  floor (`Spec::ret_floor`). Fresh literals are built directly at the expected type.
* Escape: a mutable native container that flows into arbitrary CPython code is widened to
  `object` (it is a Python object from creation), so `random.shuffle(xs)` mutates the list
  every alias sees. Read-only consumers (`native::READ_ONLY_CALLS`, e.g. `json.dumps`) get a
  copy. Calls whose argument types are not known yet are `Pending`, never misclassified as
  bridge calls (which would escape the receiver for good).
* Unpacking lowers to `ExprKind::Unpack` (a fixed tuple with an optional starred list) plus
  per-target assignments; comprehensions to `ExprKind::Comp`, built in a block expression
  with renamed (non-leaking) variables.
* `print(a, b)` evaluates all arguments before `str()` when one is mutable (as CPython).
* The planner now uses types: a method call, subscript, `for` loop or literal is a boundary
  only when it touches a Python object; it also computes reachable specializations (only
  those are emitted) and enables CPython when any reachable type holds `object`.
* Tests: `tests/differential/containers.py`, `tests/differential/escape.py`. Golden change:
  `total(xs)` in `annotations.py` is now native `total(list[int])`; diagnostics change: a
  list passed only to `json.dumps` stays native in native-only mode.

### Phase 2.2: collection performance (2026-11-22)

* Dict/set tables use an FxHash-style hasher (`rt::coll::FxHasher`) instead of SipHash.
* Method receivers, subscripts and dict keys are borrowed instead of cloned (no `Rc`
  increment/decrement per operation); comprehensions over `range(...)` iterate lazily.
* `benchmarks/collections_bench.py` (word counting + sieve), release build, wall clock with
  process startup: CPython 160 ms, native 65 ms (2.5x). The sieve half alone is 4.8x; the
  word-count half is 1.1x (dominated by allocating 300k small strings in `split`/`join`,
  which CPython's C string routines are already good at).

### Phase 2.3: formatting and variadic parameters (2026-11-22)

* `crates/runtime/src/fmt.rs`: the format-spec mini-language (`format`), printf-style `%`
  (`percent`) and `str.format` (`format_method`, including `{0!r}`, keyword fields, nested
  `{x:{w}}` and CPython's automatic/manual numbering error). Values pass as `rt::fmt::Arg`
  (`Int`/`Float`/`Bool`/`Str`/`Other`/`Py`); Python objects format through CPython.
* f-string specs and `!r` were already lowered to `format(...)`/`repr(...)` calls; those
  builtins are now native, so f-strings no longer cross into CPython.
* `def f(*args, **kwargs)`: the frontend packs extra positional arguments into
  `tuple(list)` (→ `tuple[T, ...]`) and extra keywords into `dict(**kw)` (→ `dict[str, V]`)
  at the call. `f(a, *xs)` can fill `*args`. `ExprKind::Starred` (and a `**` kwarg name)
  supports `f(*xs, **d)` on Python callables (`rt::py::call_dyn`) and `print(*xs)`.
* An empty `*args` is `tuple[Never, ...]`; consumers that need a concrete element type
  (`str.join`, `sum`) get an empty vector of that type (`iter_vec_as`).
* Tests: `tests/differential/formatting.py`, `tests/differential/varargs.py`.

### Phase 3.1: native classes (2026-11-22)

* IR: `ClassDef` (bases, methods with `MethodKind`, fields from `self.x = ...`, class
  attributes), `ExprKind::New` / `ClassRef`, `Comp::Class` / `Comp::Type`. Function
  signatures are now `CallSig` (with lowered literal defaults) and `bind_call` lives in the
  IR, so method calls bind keyword arguments and defaults after the frontend.
* Codegen: each class is a local newtype `struct Point(Rc<RefCell<PointData>>)` (identity and
  shared mutation like Python objects; a local type so runtime traits can be implemented).
  Fields are `Option<T>`: reading an unset attribute falls back to the class attribute or
  raises CPython's `AttributeError`. Generated impls: `PyFmt` (`__repr__`/`__str__`, else
  `<__main__.Point object at 0x..>`), `PyEq` (`__eq__`, else identity), `PyOrd` (`__lt__`),
  `Truthy` (`__bool__`/`__len__`), `Hash` (`__hash__`, else identity).
* Inference: instance-attribute types join program-wide (`Program::fields`), including
  widening through `self.items.append(x)`; methods are specialized functions with a typed
  `self`, resolved from the receiver type; class attributes reuse global cells
  (`Point.count`); dunders drive `+ - * ...`, comparisons, `in`, `[]`, `len`, `abs`,
  `int`, `float`, iteration (`__iter__` returning a native iterable) and unary operators.
  A read of an attribute that is never assigned anywhere compiles to a runtime
  `AttributeError` (warning P308).
* A native instance passed to CPython code is error P230 until phase 4.
* Test: `tests/differential/classes.py` (passes `--mode native-only`, no libpython).

### Phase 3.2: dataclasses, inheritance, user exceptions (2026-11-22)

* `@dataclass`: the frontend synthesizes `__init__(self, <annotated fields>)` (literal
  defaults; `field(default_factory=list|dict|set)` becomes a fresh literal inserted at each
  call, which is exactly the factory's semantics), calling `__post_init__` if defined.
  Generated `repr` is `Item(name='pen', price=1.5, ...)` and `==` compares fields. The
  `dataclasses` import is dropped when nothing uses it at run time.
* Inheritance: subclasses get their own struct; inherited methods are specialized for the
  subclass; `super().m(...)` resolves statically along the MRO; `isinstance(x, Shape)` is
  decided from the class hierarchy.
* Exceptions: a class deriving (directly or not) from a builtin exception raises as a
  native `rt::Exc` whose `kind` is the class name (CPython prints `__main__` classes
  unqualified) and whose `origin` holds the instance. `except MyError as e` binds the
  typed instance back (`e.key`); matching uses `rt::exc::register`ed (class, base) pairs.
  `str(e)` follows `BaseException`: the constructor's arguments (`''`, `str(a)`, or the
  tuple's repr), replaced by `super().__init__(msg)`.
* Each `except ... as e` binds its own local (CPython deletes the name after the handler),
  so handlers with different exception types do not box `e`.
* `type(x).__name__` of a native value is a native string. The planner skips code that a
  specialization's types decide is dead (`if False`, `x is None` with `x: None`), so dead
  branches no longer add boundaries or link libpython.
* Test: `tests/differential/classes2.py` (passes `--mode native-only`).

### Phase 3.3: polymorphism (2026-11-22)

* Joining two native class types gives a closed union `Comp::Union` (`Rect | Circle`),
  emitted as `enum U_Rect_Circle { Rect(Rect), Circle(Circle) }` with delegating `PyFmt`,
  `PyEq`, `PyOrd`, `Truthy`, `Hash` impls. A list of mixed shapes is a native
  `list[Rect | Circle]`.
* Attribute reads, method calls, dunders and attribute writes on a union `match` on the
  variant and call that class's own specialization; results are joined. `isinstance(u, C)`
  is decided statically when all variants agree, else by a `matches!` on the variant.
* Classes stay closed over the program: a union never holds a Python object.
* Test: `tests/differential/polymorphism.py` (passes `--mode native-only`).

### Phase 4.1: closures, function values, callbacks (2026-11-22)

* Defunctionalization: a function value's type names its function and the types of its
  captured variables (`Comp::Closure(f, caps)`), like a Rust closure type. Calls through a
  value are static calls of a specialization; nested functions take their environment
  (`rt::Env((cells..))`) as the first argument. Values of different functions joined in one
  place form a union (an enum) with per-variant dispatch.
* Captured variables live in shared cells (`rt::Cell<T> = Rc<RefCell<Option<T>>>`) in the
  defining function: late binding (`[lambda: i for ...]` sees the final `i`) and `nonlocal`
  writes behave like CPython; a `nonlocal` write that widens a type flows back to the
  enclosing function. Non-literal defaults of nested functions (`lambda x, i=i:`) are
  evaluated at creation and stored in extra environment slots (`EnvDefault`/`EnvRead`).
* Decorators: `@dec def f` at module level makes `f` a module variable holding `dec(f)`;
  nested `def`s apply their decorators the same way.
* Key functions and higher-order builtins (`sorted`/`min`/`max` with `key=`,
  `list.sort(key=)`, `map`, `filter`) are native; builtins passed as values in those
  positions (`key=len`) become one-argument lambdas.
* Escape: a function value reaching CPython is wrapped as a `NativeFn` pyclass
  (`rt::py::native_fn`, generated `box_<type>`), which binds Python arguments (keywords,
  defaults, CPython's TypeError texts) and calls an all-`object` specialization. Native
  exceptions become Python exceptions and back.
* Tests: `tests/differential/closures.py` (native-only), `tests/differential/callbacks.py`
  (`re.sub` with native callbacks, `functools.reduce`, `@functools.lru_cache` on a native
  recursive function, exceptions through Python). The `examples/langchain` module
  now compiles natively (its lambda is passed to LangChain as a native callable).

### Phase 4.2: forwarding wrappers (2026-11-22)

* Extra positional arguments are packed into a fixed-shape tuple (`tuple[int, str]`), so
  inside `def wrapper(*args, **kwargs)` the arity is known per specialization;
  `fn(*args, **kwargs)` expands a starred fixed tuple into its items and drops an empty
  `**kwargs` (`spec::expand_stars`, shared by inference and codegen).
* Mixed-type tuples iterate as the join of their items.
* Test: `tests/differential/wrappers.py` (logging wrapper, `@retry(3)` decorator factory
  with exceptions, stacked `@timed @logged`, `total(*[1, 2, 3])`).

### Phase 4.3: native instances in CPython (2026-11-22)

* Runtime `NativeObj` pyclass (`unsendable`, `weakref`) delegating to a generated
  `NativeObject` impl per class: `__getattr__` returns fields (boxed), properties and bound
  methods (`NativeFn` over all-`object` specializations, requested when an instance
  escapes), `__setattr__` assigns scalar fields (`FromPy`), plus `__repr__`/`__str__`/
  `__eq__`/`__hash__`. A weak-reference cache in each instance (`rt::PyCache`) keeps Python
  identity stable.
* Escape of an instance no longer errors (P230 removed): its class is recorded
  (`Program::boxed_classes`) and its mutable-container fields become Python objects (shared
  with Python). Escape records now accumulate across the local fixed point (an escape's
  trigger disappears once the escaping place has been widened).
* Fixed: `cur_fid` leaked out of class statements, making module-level lambdas after a
  class look nested in its last method. Read-only consumers (`json.dumps`) still wrap
  function values and instances passed to them (`default=to_dict`).
* Test: `tests/differential/objects_to_python.py`.

### Phase 5: rich Rust and C interop (2026-11-22)

* New `ExtTy` signature types (`crates/ir/src/hir.rs`), each with its Python type, Rust
  spelling and accepted argument types: `Vec`/`Slice`/`SliceMut` (write-back), `Opt`,
  `Tuple`, `Map`, `Struct`, `Callback` (`impl Fn`/`&dyn Fn`), `CArray` (C pointer + length,
  expanded to two C arguments), `Bytes`, `CFn` (C function pointer).
* Rust parsing (`frontend/src/externs.rs`): `pub struct` + `impl` methods (`S::name`, with
  `Self` substituted), `// pyrock: dep` lines → `[dependencies]` in the generated
  `Cargo.toml`. C parsing: `typedef struct` (with a synthesized constructor), pointer+length
  pairs, function pointer parameters (parameter lists split at top-level commas),
  `// pyrock: link` / `pkg-config` → `build.rs`.
* Codegen (`codegen-rust/src/ext.rs`): conversions both ways (`ext_out` / `conv_in`); struct
  handles are `Rc<RefCell<_>>` newtypes with Python-style printing and identity; callbacks
  are Rust closures calling the native specialization, with exceptions stashed
  (`rt::ext::stash`) and re-raised after the call; C function pointers go through a
  generated `extern "C"` trampoline that reads the current closure from a per-call-site
  thread-local stack (so re-entrant calls work).
* Buffers: any iterable converts to a slice; a mutable slice argument that is a Python
  object (`array.array`, numpy array) is written back item by item; `Vec<u8>` returns are
  Python `bytes`; `b"..."` literals lower to `bytes([...])` (hybrid).
* Handles that reach CPython are P231 (they have no Python wrapper yet).
* Tests: `tests/interop/rich/` (Rust: slices, `&mut`, `Vec` push, tuple/`HashMap`
  returns, `impl Fn`/`&dyn Fn` with closures, the `itoa` crate, a struct with methods; C:
  arrays with write-back, struct values, a function pointer integrator, `-lm`),
  `tests/interop/buffers/`.

### Phase 6.1: async / await (2026-11-22)

* Calling an `async def` makes a native coroutine (`rt::aio::Coro<T>`, type
  `Coroutine[T]`): the body is emitted as `<name>__body` and the function itself returns the
  body as a closure. `await` of a native coroutine runs it inline (as CPython does).
* When a coroutine reaches CPython (`asyncio.run`, `gather`, `create_task`), `ToPy` wraps it
  in a small Python driver coroutine plus a stackful coroutine (`corosensei`) running the
  body on its own 2 MB lazily committed stack. `await` of a Python awaitable inside the body
  suspends that stack and hands the awaitable to the driver, which awaits it in the event
  loop and resumes the body with the result or throws the exception into it. Everything
  stays on the event loop's thread, so thread-local state (globals, stdout buffer) is
  unaffected. 2000 concurrent native tasks work.
* `async with` / `async for` (`__aenter__`/`__aexit__`, `__aiter__`/`__anext__`, with
  `StopAsyncIteration`) on Python objects, and on native class instances through their
  Python wrapper. New boundary code P217 (`await` of a Python awaitable, `async with`,
  `async for`).
* Dropping a suspended coroutine (`rt::stackful::Co`) unwinds its frames with
  `panic = "unwind"`, and resets and frees the stack under `panic = "abort"` (release).
* Fixes found on the way (by the new `raylib-breakout` sample, which now compiles natively
  because `b"..."` literals are supported):
  * Functions with no `return <value>` that cannot fall off their end (always raise) have
    type `Never` instead of P306.
  * A native list/tuple/str indexed by a CPython object unboxes the index at runtime.
  * Hybrid mode: past 16 specializations a function's further direct calls share one
    specialization with `object` arguments (`Program::resolve_key`) instead of P310.
  * Diagnostics from a specialization's body are reported only when that specialization
    is reachable under the final types (`Program::spec_diagnostics`): early fixed-point
    rounds can create specializations for types that later widen.
  * Codegen stops a block at the statement where inference stops (`spec::falls_through`),
    so dead code after a statically decided `if ...: return` is not emitted.
  * `scripts/readme_check.sh` builds first, then runs with a 30 s limit; a program still
    running then (a game, a server) counts as working.
* Tests: `tests/differential/async_basic.py` (sleep, gather, exceptions, tasks,
  `wait_for` timeout), `tests/differential/async_protocols.py` (async methods, native async
  iterator and context manager classes, `asyncio.Queue`, closures, 2000 tasks, reuse error,
  uncaught exception).

### Phase 6.2: generators (2026-11-22)

* A function whose body contains `yield` (outside nested functions) is a generator: a call
  returns `rt::gen::Gen<T>` (type `Generator[T]`), and the body runs on its own stackful
  coroutine (`rt::stackful::Co`), suspending at each `yield` with the value. `T` is the join
  of the yielded types (`Never` for a generator that never yields); `return` ends the
  iteration. `yield from it` (as a statement) is `for x in it: yield x`.
* Iteration is lazy where it matters: `for` loops pull one value at a time (infinite
  generators and `break` work), as do `enumerate(gen)`, `zip` with a generator (2 or 3
  arguments) and `next(gen[, default])` (native, `StopIteration` when exhausted).
  `list`/`sum`/`sorted`/`dict`/comprehensions exhaust it. Generator methods (`def walk(self):
  yield ...; yield from child.walk()`) work.
* A generator reaching CPython is a `native_generator` iterator (`itertools.islice(gen(),
  3)`). Generators of different element types join lazily (`Gen::map`), as do coroutines
  (`Coro::map`).
* `corosensei` is now a plain runtime dependency (generators need no CPython).
* Not supported yet: `send`/`throw`/`close`, `x = yield` values, `return value`
  (StopIteration.value), async generators (P100).
* Test: `tests/differential/generators.py` (infinite generators with `break`, `yield from`,
  recursive tree walk, `enumerate(fib())`, laziness order of prints, `next` with default,
  `StopIteration`, `itertools.islice` on a native generator); also passes with `--release`
  (`panic = "abort"`, abandoned generators are reset).

### Install independence (2026-11-22)

* `crates/driver/build.rs` embeds every file of `crates/runtime` (`include_str!`, rerun on
  change) with a content hash. `runtime_path()` extracts them once into
  `<cache>/pyrock/runtime/<hash>/` (write to a temporary directory, then rename, so
  concurrent builds are safe) with a standalone `Cargo.toml` (no workspace inheritance).
  `PYROCK_RUNTIME` still overrides it.
* Checked by copying the binary out of the repo and running a program with it; every test
  suite now builds against the extracted copy.

### Demo apps (2026-11-22)

`demos/`, checked by `demos/run_all.sh` (exact stdout) and by the README check (every
README command, verbatim) in CI:

* `demos/backend`: a todo JSON API on `asyncio.start_server` with a native store module
  (classes, dataclass, user exception), native routing and query parsing (`str`
  annotations unbox the request text once), and a self-test client sending concurrent
  requests. Output identical to CPython.
* `demos/ai`: a BPE tokenizer in Python (native), a bigram language model in Rust
  (`ngram.rs`: struct handle, `HashMap`, slices, `Vec<f64>`), softmax/argmax/entropy in C
  (`sampling.c`: arrays with write-back, `-lm`). Compiles with `--mode native-only`. CPython
  reference implementations of the Rust/C modules (`reference/`) reproduce the output.
  `bench_bpe.py`: 2.7x CPython.
* `demos/gui`: 300 balls with pairwise elastic collisions (`sim.py`, native) drawn with the
  `raylib` package. `headless.py` (no window, checked exactly): 16.7x CPython. CI runs the
  window version under `xvfb-run` for 120 frames. The raylib *C API* variant was not built:
  this machine has no system raylib and building it from source was not permitted here, so
  the demo uses the Python package (drawing calls through CPython) instead.
* Fixes found by the demos:
  * `from mod import SomeClass` for a class of a native module.
  * Hybrid mode: a dict given a CPython-object key becomes a Python dict (was P304).
  * Equality between native values containing CPython objects and other native values
    (`list[object] == list[str]`) is decided by CPython (was statically `False`).
  * `x in xs` with `x: T | None` (None is never found in a list of `T`).
  * Definite assignment: synthetic `if True:` blocks (unpacking targets) run their body,
    so `for k, v in d.items()` targets are no longer "maybe unbound" (`Option` + checks).
  * One-character strings (iteration, indexing, `chr`) share 256 cached `Latin-1` values
    instead of allocating per character (BPE benchmark 1.85x → 2.7x).
* Test: `tests/differential/hybrid_mixing.py`.

### Phase 6.3: lazy generator expressions (2026-11-22)

* `(elt for t in it if c ...)` lowers to a hidden generator function
  (`def <genexpr>(__genexpr_it): for t in __genexpr_it: if c: yield elt`) called with the
  first iterable, which is evaluated in the enclosing scope as in CPython; later iterables
  and the conditions are evaluated lazily, and free variables are closure cells (late
  binding). Module-level generator expressions mark the module variables they read as
  global cells, like lambdas.
* `min`/`max(iterable, default=x)` are native.
* Test: `tests/differential/genexp.py` (infinite source with `break`, side-effect order,
  late binding, nested `for`, inside comprehensions, `next`, `itertools.islice`).

### Phase 7.1: a cheaper bridge, and threads (2026-11-22)

Measured on a breakout-style update loop (state in dicts with mixed values, so every
operation goes through CPython; 200k frames): CPython 0.61 s, pyrock **13.2 s** before.
Each bridged operation cost ~400 ns.

* The program runs attached to the interpreter (`rt::py::attached` around the module
  body): each bridge operation only bumps PyO3's attach counter, and dropped objects are
  released immediately instead of being queued in PyO3's global pending-decref pool.
  13.2 s → 2.5 s.
* `flush()` (called before every bridge operation so output stays ordered) does nothing
  when nothing was printed since (it used to lock stdout each time).
* String constants passed to CPython are interned once per call site (`rt::pystr!`), and
  native string literals are cached per call site (`rt::lit!`). 2.5 s → 1.9 s.
* Remaining gap (0.35x CPython): ~15 ns per generic C-API operation against ~5 ns for
  CPython 3.12's specialized bytecode (in-place float ops, cached dict lookups). The fix is
  typing such code natively (next item); the same loop written with classes runs 9.8x
  faster than CPython.
* Trade-off: CPython threads now run only when compiled code calls something that releases
  the GIL (I/O, `sleep`, `join`, the asyncio selector), not during long native loops.
* Threads: native functions and objects passed to CPython can be called from other threads
  (`threading.Thread(target=native_fn)`, thread pools); they panicked before. Program state
  shared across threads (module globals, the stdout buffer, the exception-class registry)
  moved from thread-locals to `rt::Shared` cells, sound because compiled code only runs
  under the GIL; per-thread execution state (recursion depth, coroutine stacks) stays
  thread-local. Generators and coroutine steps stay bound to their thread.
* Test: `tests/differential/threads.py`.

### Phase 7.2: record dicts (2026-11-22)

* A dict literal with distinct literal string keys whose values are simple types is a
  *record* (`Comp::Record`, a list of `Field { key, ty, optional }`): a generated struct
  `Rec_<hash>(Rc<RefCell<..>>)` with one typed field per key, shared like a dict. Keys
  assigned after creation are optional fields (`Option<T>` plus a per-instance insertion
  order), so `hero["prev_bottom"] = ...` keeps the record.
* Native on records: `d["k"]` reads (KeyError when absent) and writes (the field's type
  widens per key), `d.get("k"[, default])`, `"k" in d`, `len`, `del d["added"]`, printing
  and `repr` (insertion order, Python quoting), `==` (order-insensitive), truthiness,
  iterating keys (`for`, `sorted`, `list`, `min`, ...), and conversion to a Python dict when
  it reaches CPython (`json.dumps`, library calls).
* Anything else (a computed key, other methods, `.items()`, `dict(d)`) *degrades* the
  record: its place becomes a plain `dict[str, V]` through the usual widening, and a record
  that is not in a place is copied. Records with the same literal keys join key by key;
  different shapes join as dicts.
* Fixes found on the way: augmented assignment to any subscript/attribute target
  (`st["hero"]["lives"] -= 1`, receiver evaluated once); `dict.get(k)` on
  `dict[str, object]` (missing key is `None`) and `get` defaults that need boxing;
  `x in d` / `x in s` with a key of another type is `False` (was a compile error); the
  call graph uses the shared specialization past the limit, as codegen does; a request
  past the limit within one round falls back to the shared specialization.
* Measured: the breakout-style update loop (200k frames) runs in 31 ms against CPython's
  580 ms (18.8x); before this phase pyrock took 1.9 s. `raylib-breakout` goes from 3 to 43
  native functions; CPython subscripts in it drop from 280 to 19, operators from 296 to
  131 (most of the rest come from `math.sin` results, which are Python floats).
* Tests: `tests/differential/record_dicts.py`, `tests/differential/record_state_loop.py`.

### Phase 7.3: native `math`, records of different shapes (2026-11-22)

* `math.f(...)` / `from math import f` on native numbers compile to `rt::math` (sqrt, exp,
  log/log2/log10/log1p/log with base, trigonometric and hyperbolic functions, pow, fmod,
  remainder, copysign, hypot, degrees/radians, floor/ceil/trunc → int, isnan/isinf/
  isfinite/isclose, gcd/lcm, factorial, comb/perm, isqrt, ldexp, cbrt, exp2) with CPython's
  results and errors (`ValueError: math domain error`, `OverflowError: math range error`,
  `cannot convert float infinity to integer`, ...). `math.pi/e/tau/inf/nan` are constants.
  An `import math` whose every use is native is not imported at all, so such programs
  compile with `--mode native-only`.
* Records of different shapes join into one record: the keys both literals start with stay
  required, every other key becomes optional, filled in each literal's own order, so
  printing and iteration order stay exactly CPython's (a list of platforms where moving
  ones have extra `mx`/`speed` keys stays native).
* `raylib-breakout`: 49 native functions, CPython operators 296 → 103. Most of what is left
  is the raylib API itself, ctypes code in `svgload.py`, and variables that hold an `int` on
  one path and a `float` on another (boxed; a native `int | float` is the next item).
* Tests: `tests/differential/math_native.py`, more cases in
  `tests/differential/record_dicts.py`.

### Phase 7.4: `int | float` (2026-11-22)

* A value that is an int on some paths and a float on others (`x = plat["x"]` then
  `x = x + math.sin(t)`) is `rt::Num` (`I(i64) | F(f64)`, type `int | float`) instead of a
  CPython object. Arithmetic, comparisons, `abs`/`int`/`float`/`round`/`min`/`max`/`sum`,
  `math` functions, formatting and printing dispatch on the variant with exactly the int
  or float semantics (`760` stays `760`; `7.0 // 2` is `3.0`; mixed comparisons are exact),
  so it also works with `--mode native-only`.
* `raylib-breakout`: CPython operators 103 → 33, 55 native functions (the rest is mostly
  ctypes code in `svgload.py` and the raylib API).
* Test: `tests/differential/int_or_float.py`.

### Phase 6.4: the generator protocol and async generators (2026-11-22)

* Generators are `Gen<T, S, R>`: yield `T`, receive `S` (`x = yield v`; `None` from
  `next`), return `R` (`return v`). The send type is widened by `g.send(v)` calls on the
  generator's values, like a list's element type by `append`, and flows back to the
  generator function's specialization. A suspended body is resumed with a value or an
  exception: `throw(e)` raises at the `yield`, so the body's `try`/`except`/`finally` apply;
  `close()` throws `GeneratorExit` (`RuntimeError` if the generator yields again). Sending a
  non-`None` value to a just-started generator and a `StopIteration` escaping the body
  (PEP 479) raise as in CPython.
* `r = yield from sub()` delegates: values, sends and throws are forwarded and `r` is
  `sub`'s return value; `yield from` any other iterable yields its items.
* A generator that is dropped unfinished is closed (its `finally` blocks run), and a `for`
  loop owns the generator it iterates, so `break` closes it immediately, as CPython's
  reference counting does. (This also replaces the reset-without-cleanup of abandoned
  generators in release builds.) Divergence: generators still suspended in module globals
  at exit are not finalized.
* Async generators (`async def` with `yield`): the body runs as a generator yielding either
  a value or an awaitable; the consumer (`async for`, `__anext__`) awaits awaitables on its
  own coroutine stack and resumes the body with the result. `await` inside an async
  generator body, or inside a native coroutine it awaits inline, is routed to the right body
  through one stack of await targets. `aclose()`, `break` + `aclose()` running `finally`,
  and use from CPython (`anext()`, `gather`) work.
* Not supported yet: async comprehensions; `asend`/`athrow`.
* Tests: `tests/differential/generator_protocol.py`, `tests/differential/async_generators.py`
  (both also identical with `--release`).

### Phase 6.5: `asend` / `athrow` (2026-11-22)

* Async generators are `AGen<T, S>`: `x = yield v` evaluates to what `asend` passes (`None`
  for `__anext__`), with the send type widened by `asend(v)` calls as for generators. The
  body is resumed with `AIn::Value` (an `asend` value) or `AIn::Awaited` (the result of
  something it awaited).
* `agen.asend(v)`, `athrow(e)`, `__anext__()` and `aclose()` are native coroutines, so
  `await agen.asend(5)` runs inline; `athrow` raises at the `yield` (the body's `except` /
  `finally` apply); `asend(non-None)` on a just-started generator is a `TypeError` and a
  finished one raises `StopAsyncIteration`, as in CPython.
* Test: `tests/differential/async_gen_protocol.py`.

### Phase 6.6: async comprehensions (2026-11-22)

* A list/set/dict comprehension with `async for`, or with `await` in its element, key,
  value, conditions or inner iterables, lowers to a hidden `async def` that builds the
  result (`__comp_out.append(elt)` / `.add` / `[k] = v`) and is awaited in place; awaiting a
  native coroutine runs it inline, so the evaluation order and suspension points are
  CPython's. The first iterable is evaluated in the enclosing scope, as in CPython.
* A generator expression with `async for` or `await` is a hidden async generator (lazy:
  nothing runs before the first `__anext__`).
* Generator expressions and all comprehension kinds now share one lowering
  (`lower_hidden_comp`).
* Test: `tests/differential/async_comprehensions.py` (also identical with `--release`).

### Phase 6.7: finalization at exit (2026-11-22)

* Before, module-level variables were never released at exit: module-body locals were
  dropped in reverse order (and before the traceback of an uncaught exception), and global
  cells never. Generators still open at the end did not run their `finally`, and Python
  objects were not finalized (a file opened at module level and never closed lost its
  buffered data).
* Now the main module's statements run in a closure; `rt::io::module_done` prints the
  traceback of an uncaught exception first, then the module's variables are released as
  CPython's shutdown does: variables directly holding a generator first, then the others
  (containers, instances, records, Python objects that may hold one), each pass in
  first-assignment order (measured against CPython 3.12, whose shutdown collects `__main__`
  through the garbage collector). Variables shared with nested functions are emptied.
  Only variables whose release can be observed are released (generators, Python objects,
  and containers/instances that may hold them); programs with only plain native values are
  unchanged.
* Generators, async generators and coroutines now compare by identity (`g is h`).
* Test: `tests/differential/finalize_at_exit.py` (also identical with `--release`).

### Phase 6.8: finalizing imported modules (2026-11-22)

* Before, the module-level variables of an imported native module were locals of its init
  function, dropped when the import finished: its open generators were finalized at import
  time instead of at exit (and its global cells never).
* Now an imported module keeps its observable variables alive in static cells
  (`KEEP_<unit>_<name>`) and registers a finalizer (`fin_unit_<unit>`) when its import
  starts (`rt::register_module`). At exit, after the main module, `rt::io::run_main` runs
  them as CPython's shutdown does (measured on CPython 3.12): modules without reference
  cycles first, in import order, each in plain assignment order (freed by reference
  counting); then modules whose functions or classes reference their globals, in one
  garbage collection: generators held directly by variables first (all such modules, in
  import order), then the rest. The main module follows the same rule (a main without
  functions is cleared in assignment order).
* CPython's own order *among several modules with reference cycles* is not stable: it
  depends on the garbage collector's object lists and changes between the first run and
  runs with cached `.pyc` files. pyrock uses the cached-run order.
* Test: `tests/differential/finalize_modules.py` (with `_fin_*.py`; also identical with
  `--release`).

### Documentation review (2026-11-23)

* Reviewed every document against `Project.md` and the code. New
  [coverage.md](coverage.md): which `ast` nodes compile (51 of 55), and how much of real code
  is accepted as it is, measured with the new `scripts/coverage.py` (stdlib: 54.3% of
  functions and methods, 14 of 167 modules; `requests`, `huggingface_hub`,
  `langchain_core`: 32–38% of functions), with the blockers ranked.
* `how-it-works.md`: the type table now covers every representation (records, `Num`,
  classes, unions, closures, generators, coroutines) and the join rules; a new section 6b
  shows how classes, records, closures, exceptions, generators, `async def`, globals,
  native values in CPython and Rust/C imports compile, with real output; the report and
  source-map formats are described.
* `subset.md` and the skill's references: removed statements that classes, comprehensions,
  closures, `sorted`/`sum`/`round` and `with` are rejected or bridged; removed P120 (no
  longer emitted); added P002, P303, P308; listed every unsupported construct.
* Phase 8 above lists what `Project.md` asks for that isn't built, plus two known bugs found
  during the review.

### Phase 8: frame-reading builtins (2026-11-23)

* `globals()` called from compiled code made CPython return NULL without setting an
  exception, and PyO3 panicked; `eval`/`exec` without a namespace and `locals()` raised
  `SystemError: frame does not exist`. Compiled code has no Python frame for them.
* Now `globals()`, `locals()`, `vars()`/`dir()` without arguments and `eval`/`exec`
  without an explicit `globals` are rejected at compile time (P100, with a note on what to
  do). An imported `.py` that uses them is therefore imported through CPython as a whole
  (P220) and behaves exactly as in CPython. `eval(src, ns)` / `exec(src, ns)` with a
  namespace dict still run in CPython.
* Tests: `tests/diagnostics/frame_builtins.py`, `tests/differential/eval_namespaces.py`
  (with `_eval_helper.py`, which uses `globals()` and bare `eval` and is imported through
  CPython).

### Phase 8: stale specializations and the limit (2026-11-23)

* `raylib-breakout`: `draw_sky(st, bank, cam)` was specialized with `cam: object`, although
  every call passes an `int`, `float` or `int | float`. Cause: the game state is a record
  dict that gains keys over many inference rounds; every round requested a specialization
  of each `st`-taking function for the new record type, and the old ones stayed in the list.
  After 16, the limit sent calls to the shared all-`object` specialization (immutable
  arguments such as `cam` become `object`). One such round also joined `object` into the
  `sparks` record's fields for good.
* Now each specialization records the calls of its latest analysis; at the start of each
  round, specializations not reachable from the entry module are marked dead, and the
  limit counts only live specializations that existed when the round started.
* Effect on `raylib-breakout`: 55 → 63 native specializations, 241 → 223 boundaries; the
  sky, platform, actor and hero drawing do their arithmetic natively, and sparks are native
  records again.
* Test: `tests/golden/stale_specs.py` (a dict gaining a key in each of 20 functions; the
  old compiler bridged `show(st, x: object)`). `cargo test --workspace`, samples 21/21,
  demos 3/3.

### Phase 8: imports in functions and blocks, non-literal defaults (2026-11-23)

* Imports inside functions and inside module-level `try`/`if`/`with`/loop blocks: a CPython
  import there fills a slot and assigns the bound name as a variable (a Python object), in
  the scope CPython binds it in. `try: import x` / `except ImportError: x = None` therefore
  works (the variable is `object | None`). A natively compiled module (`.py`/`.rs`/`.c` next
  to the file) imported inside a function is still P100: its functions are bound for direct
  calls at module level.
* `from m import missing` now raises CPython's message, `cannot import name 'missing' from
  'm' (path)`, instead of `No module named 'm.missing'`.
* Non-literal defaults of top-level functions and methods (`def f(sep=os.sep, cache={})`):
  each becomes a hidden global, assigned where the `def` (or the class statement) runs, and
  calls that omit the argument read it. So a default is evaluated once, in definition order,
  and a mutable default is shared between calls, as in CPython. P101 no longer exists.
* Functions defined inside module-level blocks (`except ImportError: def f(...)`, `if c:
  def f` / `else: def f`) are module variables, can read module variables, and get distinct
  Rust names. A function stored in a variable that also holds a Python object gets its
  Python-callable wrapper (the generated code referred to a missing `box_fn..`).
* Two code generation bugs found on the way (the generated Rust did not compile): a lookup
  in a dict literal that never receives a key (`lookup({}, "k")`) now raises `KeyError`, and
  a Python object passed to a native `str` method (`p.replace("/", os.sep)`) is unboxed with
  a checked `TypeError`.
* `pyrock run` now replaces itself with the program (`exec` on Unix): `kill %1` or a
  service manager's SIGTERM reaches the program, and its exit status is passed on. Before,
  killing `pyrock run` left the program running.
* `scripts/readme_check.sh`: a README block that starts a background job runs as one shell
  session; a timed-out interactive program is killed with its whole process group (a
  leftover server kept the check waiting forever).
* Coverage (`scripts/coverage.py`): stdlib 54.3% → 59.7% of functions and methods, 14 → 31
  whole modules; `requests` 38.2% → 40.1%, `huggingface_hub` 37.5% → 40.1%.
* Tests: `tests/differential/nested_imports.py`, `tests/differential/default_values.py`;
  `tests/diagnostics/call_errors` no longer reports P101. `cargo test --workspace` passes;
  `ollama-agents` README session passes.

### Phase 8: the ollama-agents findings (2026-11-23)

`examples/ollama-agents/FINDINGS.md` lists problems hit while writing a real app. All
of its repros (`repro/r1.py` … `r21.py`) now pass `pyrock check --mode native-only`, and the
runnable ones print the same as CPython.

* F-02, F-03 (and F-04, already fixed): one cause. In early inference rounds a list that
  is filled later is still `list[Never]`, so a loop variable over it is `Never`; a method
  call or attribute read on it was typed `object`, and since joins never narrow, the
  variables it flowed into (`out += s.to_json()`) stayed boxed. A call or attribute read on
  a `Never` receiver is now `Never` (code generation emits the unreachable match), and it is
  not reported as a CPython boundary.
* F-01: flow-sensitive `None` narrowing. Inference tracks which `T | None` locals are known
  not to be `None` (after `if x is None: continue`/`return`/`x = v`, inside `if x is not
  None:` / `if x:`, in later operands of `and`/`or`, in the branches of `a if c else b`;
  facts are dropped for locals assigned in loops and around `try`/`with`). Each read site
  narrowed on every path is recorded on the specialization; it is typed `T` and code
  generation unwraps it. Compile-time folding of tests now also decides `if x:` on a value
  that is always `None` and `and`/`or` chains decided by a leading operand, so code that
  only runs for a non-`None` value isn't type-checked in a specialization where it's
  always `None`.
* F-05: container annotations on variables (`list[T]`, `dict[K, V]`, `set[T]`,
  `tuple[T, ...]`, `Optional[T]`, `T | None`, with class names) join into the value's type.
* Tests: `tests/differential/late_element_types.py`, `none_narrowing.py`,
  `local_annotations.py` (all also pass native-only). `cargo test --workspace`, samples
  21/21, demos 3/3.

### Phase 8: loop `else`, `:=`, `del name`, unpacking in literals (2026-11-23)

* New `crates/frontend/src/desugar.rs`, run on the AST before lowering:
  `for`/`while ... else` become a flag set before each `break` of that loop (not of nested
  loops; a nested loop's `else` does belong to it) and an `if not flag:` after the loop;
  `...` as a statement becomes `pass`; `[*a, b]`, `(*a, b)`, `{*a, b}` and
  `{**d, "k": v}` become `list(a) + [b]`, `tuple(...)`, `set(a) | {b}` and
  `dict(d) | dict({"k": v})`, evaluated left to right.
* `:=`: a new `ExprKind::Walrus` (assigns the local or global cell, evaluates to the value).
  Its targets are bindings of the enclosing scope, also from list/set/dict comprehensions
  (lowered inline), as in CPython; inside generator expressions and lambdas it is P100.
* `del name` unbinds a local or a module variable: later reads may see it unbound (the
  definite-assignment pass treats `del` as an unbinding), raising `UnboundLocalError` /
  `NameError` as CPython does; a global cell is cleared.
* A builtin applied to a value that cannot exist (`len(w)` for `w` from an empty list) is
  unreachable (`Never`) rather than a type error.
* Coverage: stdlib 59.7% → 60.7% of functions, 31 → 40 modules; `requests` 42.7%,
  `huggingface_hub` 41.8%, `langchain_core` 33.3%.
* Tests: `tests/differential/loop_else_unpacking.py`, `walrus.py`, `del_names.py` (all
  native-only where they don't use CPython). `cargo test --workspace`, samples 21/21,
  demos 3/3.

### Phase 8: packages (2026-11-23)

* A directory with `__init__.py` next to the importing file or the entry file is a package,
  and its modules are native units named by their dotted path (`app.services.billing`;
  Rust identifiers use `_`). `import a.b.c` runs each package's `__init__` once, in order,
  and binds `a`; `a.b.c.f()` resolves through the submodule map (`Loader::unit_subs`) to a
  direct call; `import a.b as x` binds the module; `from a import b` imports a submodule
  when `a` has no such name.
* Relative imports resolve against the importing unit's package (its own directory; an
  `__init__.py` is its package). A package still initialising (an `__init__` importing its
  own submodules) is referenced, not re-run, as CPython does. A relative import of a module
  file that doesn't exist raises `ModuleNotFoundError` where it runs (so `try: from
  ._speedups import f` / `except ImportError:` works).
* Re-exports: each unit's module-level bindings are recorded after its imports resolve
  (`Loader::unit_bindings`), so `from .compat import urlparse` and `compat.urlparse(...)`
  work when `compat` itself imported `urlparse` (a CPython slot, a native function, a class
  or a submodule).
* Native imports inside functions bind their names for that function (direct calls, not
  variables); native imports inside module-level blocks are resolved with the top-level
  ones.
* `scripts/coverage.py DIR` measures a package in place (each module imported by a driver,
  native-only errors minus CPython boundaries): `requests` 68.2% of functions (79.7% of
  lines), `huggingface_hub` 72.1%.
* Test: `tests/differential/packages.py` with `_pkgapp/` (nested packages, `.`/`..`
  imports, re-exports, a lazy relative import in a function, `try`/`if` imports in
  `__init__`, a missing optional module); fully native except the `json` re-export.
  `cargo test --workspace`, samples 21/21, demos 3/3.

### Phase 8: abstract base classes (2026-11-23)

* `class C(ABC)`, `class C(abc.ABC)` and `class C(metaclass=ABCMeta)` compile as plain native
  classes (`ABC` may be combined with a native base); `@abstractmethod` / `@abc.abstractmethod`
  (also under `@property`, `@staticmethod`, `@classmethod`) marks a method abstract, and its
  body stays callable through `super()`.
* Creating an instance of a class whose MRO leaves abstract methods unimplemented raises
  CPython's `TypeError: Can't instantiate abstract class C without an implementation for
  abstract method(s) 'a', 'b'` at that point; classes are closed, so this is decided at compile
  time (`Module::unimplemented_abstracts`).
* `abc` joined the modules whose imports are dropped when only used this way, so such
  programs need no CPython. `ABC`, `ABCMeta` and `abstractmethod` are recognised from the
  module's own `abc` imports, because classes are declared before imports resolve (a module
  imported back by a package import cycle must already expose its classes; declaring them
  later dropped `requests` from 68% to 62%).
* Test: `tests/differential/abstract_classes.py` (native-only).

### Phase 8: dataclass options (2026-11-23)

* `@dataclass(frozen=, order=, eq=, repr=, kw_only=, slots=)` with literal booleans
  (`slots`, `init`, `match_args`, `weakref_slot` have no observable effect natively).
  `frozen=True`: assigning a field outside the generated `__init__` raises
  `dataclasses.FrozenInstanceError` (an `AttributeError`), and with `eq` instances hash by
  their hashable fields, so equal frozen instances are the same dict key or set member.
  `order=True`: `<`, `<=`, `>`, `>=`, `sorted` and `min`/`max` compare the fields in order,
  like tuples. `eq=False` / `repr=False` keep identity equality and the default repr.
  `kw_only=True` makes every field keyword-only.
* Test: `tests/differential/dataclass_options.py` (native-only).

### Phase 8: enums (2026-11-23)

* `class C(Enum)` / `IntEnum` / `StrEnum` (from `enum`, or `enum.Enum`) without its own
  `__init__` is rewritten by the AST dump (`dump_ast.py`) into a plain class: members are
  instances created in the class body (`RED = C("RED", 1)`, `auto()` numbered from the last
  int, or the lowercased name for `StrEnum`; a repeated value is an alias of the first
  member), with `name`/`value` properties, CPython's `repr` (`<C.RED: 1>`) and `str`
  (`C.RED`; the value for `IntEnum`/`StrEnum`), a member list and lookup helpers.
  `IntEnum`/`StrEnum` also compare with plain values, and `IntEnum` adds `+`, `-`, `*`,
  `int()`.
* The frontend maps uses of the class itself: `C(v)` looks a member up by value (`ValueError:
  v is not a valid C`), `C["NAME"]` by name (`KeyError`), and `for m in C`, comprehensions,
  `list`/`len`/`sorted`/... and `m in C` go over the members; also through `mod.C`.
  Members are singletons, so `is`, `==` and dict keys work by identity.
* Compile-time folding decides `isinstance(x, C)` for a local that is exactly a `C` or a
  builtin value, so methods with `isinstance` dispatch are typed per specialization.
* Tests: `tests/differential/enums.py`, `enums_int_str.py` (native-only).

### Phase 8: coverage after packages, ABCs, dataclass options and enums (2026-11-23)

* `scripts/coverage.py`: stdlib 62.4% of functions and methods (41 modules); `requests`
  68.2%, `huggingface_hub` 73.1%, `langchain_core` 41.4%, each measured in place. Ranked
  blockers in [coverage.md](coverage.md).

### Phase 8: typing decorators (2026-11-23)

* `@override` and `@final` (from `typing` / `typing_extensions`, by name or as `typing.X`)
  are dropped and `@overload` stubs removed before lowering (`desugar.rs`): they only inform
  type checkers, and CPython keeps the last, real definition. `langchain_core` had 420
  `@override` and 77 `@overload` uses.
* Coverage: `requests` 68.2% → 74.9%, `huggingface_hub` 73.1% → 75.0%, `langchain_core`
  41.4% → 43.5%; stdlib unchanged at 62.4%.
* Test: `tests/differential/typing_decorators.py` (native-only).

### Phase 8: rename to pyrock (2026-11-23)

* The project is now **pyrock** (the name pyfast is taken): crates (`pyrock-*`), the binary,
  `.pyrock/` output folders, `~/.cache/pyrock`, `PYROCK_TARGET_DIR` / `PYROCK_RUNTIME`,
  `// pyrock:` directives in `.rs`/`.c` files, docs, the skill (`skills/pyrock`, linked into
  the Claude Code, Codex and Cursor skill folders) and the user samples. The repository
  directory keeps its path. Installed globally with `perch install` as `pyrock`.

### Phase 8: cached_property, NamedTuple, iterating native instances (2026-11-23)

* `@functools.cached_property` (by name or `functools.cached_property`) is rewritten by the
  AST dump into a property that computes once per instance and stores the value in an
  instance attribute.
* `class P(typing.NamedTuple)` is rewritten into a frozen, ordered dataclass with `len`,
  indexing (negative too, `IndexError` past the end), iteration, `_asdict()` and `_fields`.
  Equality with plain tuples and `isinstance(p, tuple)` are not supported.
* Unpacking a native instance (`x, y = p`) and passing one to a builtin that iterates its
  argument (`list`, `tuple`, `sorted`, `sum`, `min`, `max`, `zip`, ...) go through its
  `__iter__` natively. Such a call stays pending until `__iter__`'s type is known, instead
  of being sent to CPython in an early inference round (which made the class escape and
  its field types `object` for good).
* `functools` joined the imports dropped when unused after rewriting.
* Tests: `tests/differential/cached_property.py`, `namedtuples.py` (native-only).

### Phase 8: multiple inheritance (2026-11-23)

* A class can have several natively compiled bases (`class User(JsonMixin, ReprMixin, Base)`),
  plus at most one builtin exception root. `ClassDef::bases` keeps them in order and the
  method resolution order is CPython's C3 linearisation (`c3_mro`), used for method and
  class-attribute lookup, fields, `isinstance` and `super()`.
* `super().m()` takes the next class after the defining class in the receiver's MRO. When
  some subclass continues the MRO differently (a diamond: `class D(B, C)` makes `super()`
  in `B` reach `C`), the call is a `SuperDispatch` whose alternative is picked from the
  receiver's type in each specialization, so cooperative multiple inheritance matches
  CPython.
* Test: `tests/differential/mixins.py` (mixins, `super()` chains, a diamond; native-only).

### Phase 8: typing constructs at runtime (2026-11-23)

* With names from `typing` / `typing_extensions`, the AST dump drops `Protocol` and
  `Generic[...]` bases, removes `TypeVar` / `ParamSpec` / `TypeVarTuple` assignments,
  turns `X = NewType("X", t)` into an identity function, and treats a `TypedDict` class as
  what it is at runtime: its class statement is removed and `Movie(title=..., year=...)`
  becomes a dict literal (a native record dict).
* Field types are joined program-wide, so a `Generic[T]` container used with several `T`s
  keeps its field as a Python object (hybrid mode); native-only reports it (currently with
  a misleading P306 message, listed above).
* Test: `tests/differential/typing_runtime.py`.

### Phase 8: forwarding `**kwargs` (2026-11-23)

* `f(..., **mapping)` into a native function with its own `**kwargs` parameter, when the
  call binds every named parameter itself (so no key could fill one): the mapping is merged
  into the callee's kwargs dict, as in callback plumbing such as `handle_event(handlers,
  name, *args, **kwargs)`. A call that would leave a named parameter for the mapping is
  still P100.
* Test: `tests/differential/kwargs_forwarding.py`.

### Phase 8: coverage after typing constructs, mixins and kwargs forwarding (2026-11-23)

* stdlib 62.4% → 64.1% of functions and methods (42 modules); `requests` 74.9% → 82.8%,
  `huggingface_hub` 75.0% → 75.1%, `langchain_core` 43.5% → 49.5%.

### Publishing: examples, installation, ignore rules (2026-11-23)

* The user samples moved from the private `.ignore/samples/` into `examples/`, with an
  overview README, a pinned `requirements.txt` for the virtualenv (`examples/.venv`) and
  `build.sh` scripts that download or build what `raylib`, `raylib-breakout` and
  `opencv-yolo` load (raylib 5.5 release, ONNX Runtime 1.32.0 from its PyPI wheel,
  OpenCV 4.10.0, YOLO26 ONNX exports). Hardcoded absolute paths became relative to the
  source file (`__FILE__`) or the loaded library (`dladdr`). Binaries, third-party
  downloads, models and videos are ignored by git.
* README: an Installation section (`cargo install --path crates/driver`, requirements, a
  first program). The README check skips examples marked `skip-ci` when `CI` is set.
* Golden snapshots replace the build machine's `sys.path` with a placeholder, so they pass
  on any machine.
* Fix: recursion through a conditional expression (`return n if n < 2 else fib(n - 1) +
  fib(n - 2)`) failed with P306; the expression now keeps the typed branch while the other
  is still unknown. Test: `tests/differential/recursive_ifexp.py`.

### Releases and the website (2026-11-23)

* `.github/workflows/release.yml`: pushing a `v*` tag builds `pyrock` for
  `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` (static) and
  `aarch64-apple-darwin`, runs `scripts/release_smoke.sh` with each binary on its own
  platform (a native-only program; a hybrid one importing `json`, a `.rs` and a `.c` file;
  `explain`), and publishes the archives that passed, with `SHA256SUMS`, as a GitHub
  release. Verified locally: the static x86-64 build passes the smoke test.
* `install.sh`: picks the archive for the platform, checks its SHA-256 and installs into
  `~/.local/bin` (`PYROCK_INSTALL_DIR`, `PYROCK_VERSION`).
* `LICENSE-MIT` and `LICENSE-APACHE`, matching the `MIT OR Apache-2.0` already declared in
  `Cargo.toml`.
* README: prebuilt and source installation, and a Usage section (workflow, options, output
  layout, diagnostics, environment).
* `site/index.html`: the project website (how it works, install, usage, speed, comparison,
  examples, roadmap), published to GitHub Pages by `.github/workflows/pages.yml`. Every code
  sample on it is real output of pyrock 0.1.0.
* v0.1.0 was published with the two Linux binaries only: GitHub no longer runs the
  `macos-14` label, so the macOS job never started. The workflow now uses `macos-15`, and
  v0.1.1 (same compiler, version bump only; the four golden snapshots differ only in their
  version header) is the first release with every target.

### The playground (2026-11-23)

* `site/play.html`: write Python and compile it in the browser. Parsing runs pyrock's own
  `dump_ast.py` in Pyodide 0.27.7 (CPython 3.12.7 in WebAssembly); the rest of the compiler
  runs as `crates/playground` built for `wasm32-unknown-unknown` (2 MB, no imports, no
  generated glue). It shows `check`, `explain` and the generated `main.rs`. Run executes the
  program with CPython in Pyodide, because building the Rust needs Cargo; the page says so.
  Both run in a Web Worker, so Stop ends a program that never finishes.
* Frontend: `parse_ast_json` compiles a file from an AST produced elsewhere, without
  reading the disk (imports go to CPython). `explain`'s text moved to
  `analysis/src/explain.rs`, shared by the CLI and the playground (explain goldens
  unchanged).
* Verified: `crates/playground/tests/compile.rs` (specializations, boundaries,
  native-only errors, syntax errors, single-file imports). In Node, the wasm's Rust and
  explain output for `gen/main.py` are byte-identical to the CLI's (apart from the version
  header). In headless Chrome (driven over the DevTools protocol), all five examples
  compile and run, native-only turns boundaries into errors, Stop ends an infinite loop,
  share links round-trip, and the console shows no errors.

* Syntax highlighting in the playground editor: a highlighted copy of the text under a
  transparent textarea (typing, selection, undo and mobile keyboards stay native), with a
  Python tokenizer for keywords, builtins, strings (prefixes, triple-quoted, unterminated),
  numbers, comments, decorators and definition names. Checked in headless Chrome: the copy
  matches the textarea's text exactly, glyphs line up (drawn over each other), and
  scrolling and Tab stay in sync.
* Pictures in the playground: the Output tab draws what a program prints. One SVG is shown
  as an image; HTML renders in a sandboxed frame (`allow-scripts`, opaque origin); several
  SVGs in a row, or `P3` PPM images, play as frames on an image or canvas with
  play/pause, a frame slider and a speed setting. It is ordinary text output, so a compiled
  program prints the same picture. Five examples (bar chart, bouncing balls, Mandelbrot,
  Game of Life, HTML report) compile with `--mode native-only`; they are also differential
  tests (`tests/differential/playground_*.py`), so their output stays identical to
  CPython's. The worker sends output in 64 KB batches instead of one message per `print`.
  Checked in headless Chrome: each example is detected and drawn (canvas pixels set,
  animations advance), the Picture/Text toggle works, and a later plain-text run clears
  the picture.

### One `examples/` tree (2026-11-23)

* `samples/`, `demos/` and `examples/` held overlapping kinds of programs. Everything
  runnable is now under `examples/`, grouped by what it shows: `tour/` (was `samples/`,
  plus the two native-only examples, now 22 and 23), `interop/`, `packages/`, `apps/`
  (was `demos/`, plus `ollama-agents`), `graphics/` and `playground/`. `examples/hello`
  and `examples/generics.py` were copies of `tour/01_hello.py` and `tour/06_generics.py`
  and are gone. `benchmarks/` and `tests/` stay as they are.
* The playground's programs live in `examples/playground/` (one copy): the website loads
  them (`scripts/build_site.sh` copies them and builds the wasm), and the differential test
  runs them. That test now honours `# mode: native-only` (before, it always compiled in
  hybrid mode) and checks `# expect: compile error (Pxxx)`.
* `scripts/readme_check.sh` runs `examples/<group>/<example>/README.md`; `interop/imports`
  got a README. Verified: `cargo test --workspace`, `examples/tour/run_all.sh` (23/23),
  `examples/apps/run_all.sh` (3/3), `CI=1 scripts/readme_check.sh` (11 passed, 6 skipped),
  the playground in headless Chrome, and every GitHub link on the home page.

### Bigger playground programs (2026-11-23)

* Six more programs in `examples/playground/`, all `--mode native-only`, each a
  differential test: a ray tracer (PPM), a Lisp interpreter (classes as one Rust enum,
  closures, recursion), maze generation with an animated breadth-first search (PPM
  frames), flocking birds (SVG frames), two chaotic double pendulums (SVG frames) and an
  L-system plant (SVG). Their headers give measured times (for example the ray tracer:
  30 ms compiled with `--release`, 321 ms in CPython 3.12). The playground lists them as
  "Bigger programs".
* Writing them found four compiler problems, now under "Known bugs" above with repros in
  `tests/known_bugs/`; the programs work around them.

### Phase 8: `isinstance` narrows unions of classes (2026-11-23)

* A function specialized for one class now decides `isinstance(v, C)` by the class
  hierarchy either way (it only decided "exactly `C`" before), so the branch that can't
  run in that copy isn't type-checked. This also covers unions whose members all agree
  and tuples of classes (`isinstance(v, (A, B))`).
* Flow-sensitive narrowing of unions of native classes, next to the existing `None`
  narrowing: each branch of `if isinstance(v, C)` (also `elif`/`else`, `not`, `and`/`or`,
  `while`, conditional expressions) knows which classes `v` can still be, and joins,
  loops, `try` and assignments forget it as they do for `None`. Only reads that are the
  object of an attribute or method access (`v.side`, `v.area()`) are narrowed, to one class
  or a smaller union; codegen converts the union value at those reads. A narrowed value
  flowing into a list or variable keeps the union type, so containers keep one element
  type.
* A class instance stored where `T | None` or a union containing `T` is expected no longer
  widens the place it came from (`Env(f.env)` made the field `Fn.env` optional, which then
  spread to everything using it). Lists and dicts still widen as before.
* Codegen: a `range` bound containing a block (`len(v.xs)` on a narrowed `v`) is
  parenthesized, so `for` doesn't parse it as the loop body.
* The HIR walkers (`walk_stmts`, `walk_block`, `walk_expr`) moved from `analysis` to
  `ir::walk` (re-exported from `analysis`), so inference can use them.
* Verified: `tests/differential/isinstance_narrowing.py` (native-only; every form above,
  including a reassignment in the loop) matches CPython; both repros from
  `tests/known_bugs` now compile natively and match CPython (the Lisp interpreter:
  21 native specializations, no warnings; it produced Rust that didn't compile before);
  `cargo test --workspace`, the tour (23/23) and the apps (3/3) pass, golden snapshots are
  unchanged, and `raylib-breakout` checks exactly as before (63 native, 230 boundaries).

