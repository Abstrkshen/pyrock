# Roadmap: Python for real apps

Goal: compile enough of the Python that **backend, AI and GUI apps** actually use, and make
existing **C and Rust projects** easy to use from it. Not all of Python: anything dynamic
keeps running in the embedded CPython, at a reported boundary.

This file tracks progress. Each step has an acceptance gate, and the log at the bottom
records what changed and how it was verified. `Project.md` §13 milestone M6 ("collections,
classes, closures, callbacks") is split into the phases below.

Status: `[ ]` not started · `[~]` in progress · `[x]` done (gate met).

## Phase 0: Basics

- [x] git repository with an initial commit
- [x] Relative `--python` path works (regression test `relative_python_path`)
- [x] Generated Rust compiles without warnings (`dead_code` was leaking into `pyfast run` output)
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
- [ ] `@dataclass`
- [ ] Single inheritance and `super()`
- [ ] User-defined exception classes
- [ ] Dynamic classes (metaclasses, `__getattr__`, `setattr`) fall back to CPython

## Phase 4: Closures and callbacks

- [ ] Lambdas, nested functions, captured variables, `nonlocal`
- [ ] Functions as values (`Fn` type), passing native functions to native functions
- [ ] Native functions callable from Python (callbacks, decorators such as `@app.get`)
- [ ] `f(*args, **kwargs)` forwarding into a native function's fixed parameters (wrappers)

## Phase 5: Rich Rust and C interop

- [ ] Lists and slices (`Vec<T>`, `&[T]`, `T *` + length) across the boundary
- [ ] Buffers (`bytes`, buffer protocol, numpy arrays)
- [ ] Structs ↔ classes; Rust `impl` methods; opaque handles
- [ ] Callbacks (function pointers, `Fn` arguments)
- [ ] Cargo dependencies for `.rs` files; library / pkg-config flags for `.c` files

## Phase 6: async

- [ ] `async def` / `await` bridged to asyncio

## Throughout

- [ ] Install without depending on the repo path (runtime crate shipped with the binary)
- [ ] Demo apps compiled and checked in CI: backend (JSON API handlers), AI (tokenizer +
      C/Rust inference glue), GUI (raylib through its C API)

## Log

### Phase 0 (2026-11-22)

* `git init`, `.gitignore` (target, `.pyfast/`, `__pycache__`, `.ignore/`, built `.so`).
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
* Effect: `.ignore/samples/rust-http/client.py` (uses `try`/`with`) now compiles natively
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
