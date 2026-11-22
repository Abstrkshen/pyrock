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
- [ ] `try` / `except` / `else` / `finally`, `raise`, exception hierarchy for builtin types
- [ ] `with` (CPython context managers; native ones once classes land)
- [ ] Default parameter values, keyword arguments to native functions
- [ ] Tuple unpacking (`a, b = f()`, `for k, v in ...`)
- [ ] Module-level constants readable inside functions (today P140)

## Phase 2: Native collections

- [ ] `list[T]` as a shared handle (`Rc<RefCell<Vec<T>>>`), so aliasing matches CPython
- [ ] `dict[K, V]` (insertion-ordered), `tuple`, `set[T]`
- [ ] Methods, `len`, `in`, indexing, slicing, iteration
- [ ] Builtins: `enumerate`, `zip`, `sorted`, `sum`, `reversed`, `list()`, `dict()`
- [ ] List / dict / set comprehensions (inlined as loops)
- [ ] Escape rule: a collection that reaches Python is a Python object from creation

## Phase 3: Classes

- [ ] Classes with `__init__`, fields inferred from `self.x = ...`, methods
- [ ] `@dataclass`
- [ ] Single inheritance and `super()`
- [ ] User-defined exception classes
- [ ] Dynamic classes (metaclasses, `__getattr__`, `setattr`) fall back to CPython

## Phase 4: Closures and callbacks

- [ ] Lambdas, nested functions, captured variables, `nonlocal`
- [ ] Functions as values (`Fn` type), passing native functions to native functions
- [ ] Native functions callable from Python (callbacks, decorators such as `@app.get`)

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
