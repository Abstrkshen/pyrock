# How pyfast works

This document follows one program from `pyfast run main.py` to a native executable. It covers
what each stage does, the decisions it makes and the code it produces. All code samples are
real compiler output.

```
main.py ──► 1. parse (CPython ast → JSON)
        ──► 2. lower + resolve imports  → HIR
        ──► 3. infer generic signatures (for explain)
        ──► 4. specialize: one copy per concrete type combination
        ──► 5. plan: tiers, effects, CPython boundaries
        ──► 6. emit Rust + Cargo.toml (+ build.rs for C)
        ──► 7. cargo build → executable
```

Each stage is a crate under [crates/](../crates/):

| Stage | Crate | Main entry point |
|---|---|---|
| 1–2 | `frontend` | `parse_file` in [lib.rs](../crates/frontend/src/lib.rs), [dump_ast.py](../crates/frontend/src/dump_ast.py), [externs.rs](../crates/frontend/src/externs.rs) |
| — | `ir` | HIR, types, spans, diagnostics |
| 3–4 | `infer` | [generic.rs](../crates/infer/src/generic.rs), [spec.rs](../crates/infer/src/spec.rs), [ops.rs](../crates/infer/src/ops.rs), [definite.rs](../crates/infer/src/definite.rs) |
| 5 | `analysis` | `plan` in [lib.rs](../crates/analysis/src/lib.rs) |
| 6 | `codegen-rust` | `emit` in [lib.rs](../crates/codegen-rust/src/lib.rs) |
| runtime | `runtime` | Python semantics in Rust and the PyO3 bridge, linked into every program |
| 7 | `driver` | The `pyfast` CLI in [main.rs](../crates/driver/src/main.rs) |

The running example:

```python
import json
import geometry          # geometry.py next to it
import vecmath           # vecmath.rs next to it

def add(a, b):
    return a + b

def label(n: int) -> str:
    return "n=" + str(n)

print(add(1, 2), add(1.5, 2.0), add("a", "b"))
print(label(json.loads("7")))
print(vecmath.dot(1.0, 2.0, 3.0, 4.0), geometry.circle_area(1.0))
```

---

## 1. Parsing: CPython does it

pyfast doesn't have its own Python parser. It runs the real interpreter in isolated mode
(`python3 -I dump_ast.py main.py`), which parses the file with the `ast` module and prints the
tree as JSON (protocol version 1). Every node carries its location, and constants are tagged
so nothing is lost on the way. Integers are sent as strings, so a huge literal can still be
reported, and floats are sent as their `repr`.

This means syntax is exactly whatever the installed Python accepts, and syntax errors are
CPython's own messages (P001).

## 2. Lowering to HIR and resolving imports

The frontend turns the JSON tree into **HIR**, a smaller, scope-resolved representation
defined in [hir.rs](../crates/ir/src/hir.rs). During this step it:

* finds every top-level function and resolves every name to a local, a function, a builtin,
  an imported module or a CPython slot. An undefined name is reported here (P143);
* rewrites sugar into simpler forms: f-strings become formatting calls, `x[a:b]` becomes
  a `slice()` call, `assert` becomes `if not c: raise AssertionError`, list/set/dict
  comprehensions become loops in their own scope, generator expressions and async
  comprehensions become hidden generator or async functions, chained comparisons and augmented assignment to complex
  targets get temporaries so each operand is evaluated once;
* collects classes (fields from every `self.x = ...`, methods, properties, bases), nested
  functions with the variables they capture (which become shared cells), and generator and
  async functions;
* records annotations (`a: int`, `-> str`) on parameters and returns;
* **resolves imports** (see [imports.md](imports.md)):
  * `x.py` is loaded recursively as another **unit** (parsed and lowered the same way) and
    gets an `init_x` function for its module body;
  * `x.rs` is parsed with `syn`, and every `pub fn` signature is mapped to an `ExternFn`;
  * `x.c` (or `x.h`) is scanned for function prototypes, and C types are mapped the same way;
  * anything else becomes a **slot**: a placeholder for a module or attribute that CPython
    imports at runtime.

Unsupported constructs (`match`, `:=`, a class with a CPython base class, an import inside a
function, ...; the full list is in [coverage.md](coverage.md)) are rejected here with a code
and location. For an imported `.py` in hybrid mode, the whole file is imported through
CPython instead (P220).

## 3. Generic signatures

pyfast never asks you for types. For each function it infers a **generic scheme** in the
style of Hindley–Milner, where each operator adds a constraint:

```
add(A, B) -> R where Add(A, B, R)
label(int) -> str
geometry.circle_area(A) -> R where Mul(float, A, R1), Mul(R1, A, R)
```

`Add(A, B, R)` means "whatever `A + B` gives for these types is `R`". These schemes are what
`pyfast explain` shows as *Inferred generic signatures*. Annotated parameters (`n: int`) are
fixed instead of generic.

The schemes explain behaviour. Code generation uses the next stage.

## 4. Specialization: one Rust function per type combination

Starting from the module body, pyfast walks every **reachable** call and records the concrete
argument types. Each new combination (function, argument types) becomes a **specialization**
and is analysed in turn, until nothing new appears (a fixed point).

```
add(1, 2)        → add(int, int)     → add__int_int
add(1.5, 2.0)    → add(float, float) → add__float_float
add("a", "b")    → add(str, str)     → add__str_str
```

Inside each specialization every local has one concrete type, computed from the **operator
table** in [ops.rs](../crates/infer/src/ops.rs). That table encodes Python's rules, for
example `int + float → float`, `int / int → float`, `str * int → str` and `bool + bool → int`.
If an operator is applied to types where CPython would raise `TypeError`, that is a compile
error (P301).

The types ([types.rs](../crates/ir/src/types.rs)) and their Rust representation:

| Python | Rust |
|---|---|
| `int` | `i64` (checked, see below) |
| `float` | `f64` |
| `bool` | `bool` |
| `str` | `rt::Str` (`Rc<str>`, so cheap to clone) |
| `None` | `()` |
| `T \| None` | `Option<T>` |
| `int \| float` | `rt::Num` (an enum: `I(i64)` or `F(f64)`) |
| `list[T]`, `dict[K, V]`, `set[T]` | `rt::List<T>`, `rt::Dict<K, V>`, `rt::Set<T>`: shared `Rc<RefCell<..>>` handles, so aliases see each other's changes |
| `tuple[A, B]`, `tuple[T, ...]` | a Rust tuple, `rt::Tup<T>` |
| a dict with literal string keys | a generated struct with one typed field per key (a *record*) |
| an instance of class `C` | a generated `C(Rc<RefCell<CData>>)` newtype |
| one of several classes | a generated enum with one variant per class |
| a function or closure | a function item, or `rt::Env<(captured cells..)>` for a closure; several different functions in one place become a generated enum |
| a generator, async generator, coroutine | `rt::gen::Gen<Yield, Send, Return>`, `rt::agen::AGen`, `rt::aio::Coro<T>` |
| a list with no elements yet | `rt::List<rt::Never>` until something is added |
| anything else | `rt::py::Obj`, a Python object |

Types are interned (`Ty` is a small `Copy` handle), so specialization keys stay cheap to
compare and hash. When two paths give different types, the join tries, in order: the same
type; `None` + `T` → `T | None`; `int` + `float` → `int | float`; two records → a record
with the common keys required and the rest optional; two classes → a union; containers
elementwise. Anything else is `object` (boxed, P215).

Details that matter:

* **Recursion.** A recursive call to a specialization that is still being analysed uses its
  return type so far, and the loop runs until the types stop changing.
* **Conflicts.** If a variable or return value has two different types (`return n * 2` in one
  branch, `return None` in another), hybrid mode **boxes** it: the value becomes a Python
  object (P215). Native-only mode reports an error instead.
* **Maybe-unbound locals.** A definite-assignment pass ([definite.rs](../crates/infer/src/definite.rs))
  finds variables that might be read before assignment. These are stored as `Option<T>`, and
  reading `None` raises `UnboundLocalError` exactly where CPython would.
* **Limits.** A function can have at most 16 live specializations. Further calls with new
  types share one all-`object` specialization in hybrid mode (bridged), and are an error
  (P310) in native-only mode. Only specializations still reachable from the entry module
  count: when a parameter type widens over several rounds (a dict gaining keys), each round
  leaves a stale specialization behind, and those don't use up the limit. Unreachable functions are never compiled.
* **Program-wide facts** are joined inside the same fixed point: the type of each class
  field (every `self.x = ...` in the program), each module global, each record key, and
  what a generator receives through `send`.
* **Annotations.** When a Python object flows into an `int`/`str`/`bool` annotation, it's
  unboxed to the native type, and `TypeError` is raised if the object is the wrong type. This
  is how `label(json.loads("7"))` becomes `label__int`.

## 5. The plan: tiers, effects and boundaries

The analysis crate looks at every specialization and decides:

* **Tier.** *Native* means pure Rust. *Bridge* means it touches Python objects.
* **Effects.** These are `io` (prints), `may_raise` (for example, checked int arithmetic can
  overflow), `ffi` (calls Rust/C) and `calls_python`. `explain` shows them.
* **Recursion.** Functions on a call-graph cycle get a frame guard that raises
  `RecursionError` at depth 1000.
* **Boundaries.** Every place native code crosses into CPython, with its location and reason:

| Code | Boundary |
|---|---|
| P200 | runtime import of a CPython module |
| P210 / P211 | attribute access, method call or call of a Python object / builtin |
| P212 | list, dict, tuple or set literal, subscript, item assignment |
| P213 | operator or comparison involving a Python object |
| P214 | `for x in <python iterable>` |
| P215 | value boxed because it has no single native type |
| P216 | `with` on a Python context manager, `except` naming a Python exception class |
| P217 | `await` of a Python awaitable, `async with` / `async for` on Python objects |
| P220 | (warning) a local `.py` that can't be compiled is imported through CPython |

In **hybrid mode** (the default), boundaries are allowed and listed. In **native-only mode**
each one is a compile error, so a program that compiles in native-only mode never touches
Python.

## 6. Code generation

Each specialization becomes one plain Rust function. For the example:

```rust
/// `add(a: int, b: int) -> int`, specialized from demo.py:5
fn add__int_int(mut a: i64, mut b: i64) -> PyResult<i64> {
    return rt::int::add(a, b);          // checked: overflow → OverflowError
}

/// `add(a: float, b: float) -> float`, specialized from demo.py:5
fn add__float_float(mut a: f64, mut b: f64) -> PyResult<f64> {
    return Ok((a + b));
}

/// `add(a: str, b: str) -> str`, specialized from demo.py:5
fn add__str_str(mut a: rt::Str, mut b: rt::Str) -> PyResult<rt::Str> {
    return Ok(rt::string::concat(&a, &b));
}
```

Every function returns `PyResult<T>`, which is `Result<T, Exc>`. A Python exception is an
`Err` that `?` propagates up the stack. That's how `raise` semantics work without unwinding.

**Imports in the generated code:**

```rust
// vecmath.rs is compiled as a submodule of the generated crate
#[path = "/abs/path/vecmath.rs"]
mod rs_vecmath;

// C: declarations from fastc.h; build.rs compiles fastc.c
mod c_fastc {
    extern "C" {
        pub fn gcd(a0: i64, a1: i64) -> i64;
        pub fn scale(a0: f64, a1: std::ffi::c_int) -> f64;
        pub fn version() -> *const std::ffi::c_char;
    }
}

// CPython imports: a lazily filled slot per imported name
static SLOT_0_json: rt::py::Slot = rt::py::Slot::new("json");

// geometry.py: its module body, guarded so it runs once
static INIT_geometry: AtomicBool = AtomicBool::new(false);
fn init_geometry() -> PyResult<()> {
    if !rt::first_import(&INIT_geometry) { return Ok(()); }
    ...
}
```

And the module body:

```rust
fn module_body() -> PyResult<()> {
    SLOT_0_json.import("json", true)?;          // CPython import
    init_geometry()?;                           // native module body
    rt::print(&[rt::conv::str_int(add__int_int(1_i64, 2_i64)?), ...]);
    // json.loads("7") is a CPython call; the annotation unboxes the result to i64
    rt::print(&[label__int(rt::py::unbox_annotated::<i64>(
        &rt::py::call(&rt::py::getattr(&SLOT_0_json.get()?, "loads")?,
                      vec![rt::py::str_(&rt::Str::from("7"))], vec![])?, "int")?)?]);
    // Rust and native-Python calls are direct function calls
    rt::print(&[rt::conv::str_float(rs_vecmath::dot(1.0_f64, 2.0_f64, 3.0_f64, 4.0_f64)),
                rt::conv::str_float(geometry_circle_area__float(1.0_f64)?)]);
    Ok(())
}

fn main() {
    rt::py::init("demo.py", &[/* sys.path captured at build time */]);
    rt::run_main("demo.py", module_body)
}
```

Arguments to C are converted with checks (`rt::ext::int_to::<c_int>(x, ...)?` raises
`OverflowError`), and returned `char *` is copied with `rt::ext::from_cstr`. A boxed value
passed to a native parameter is unboxed with `rt::py::to_i64(&obj)?` and similar calls.

**A boxed function.** When types conflict in hybrid mode, the code works on `Obj`:

```python
def find(n):
    if n > 3:
        return n * 2
    return None
```

```rust
/// `find(n: object) -> object`
fn find__obj(mut n: rt::py::Obj) -> PyResult<rt::py::Obj> {
    if rt::py::truthy(&rt::py::compare("gt", n.clone(), rt::py::int(3_i64))?)? {
        return rt::py::binop("mul", n.clone(), rt::py::int(2_i64));
    }
    return Ok(rt::py::none());
}
```

Names that are Rust keywords are escaped (`type` → `r#type`, `_` → `py__`).

## 6b. How the larger features compile

All excerpts below are compiler output for this program (shortened where marked):

```python
class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y

    def norm2(self):
        return self.x * self.x + self.y * self.y

def make_counter():
    n = 0
    def inc():
        nonlocal n
        n += 1
        return n
    return inc

def countdown(n):
    while n > 0:
        yield n
        n -= 1

def safe_div(a, b):
    try:
        return a / b
    except ZeroDivisionError:
        return -1.0

p = Point(3, 4)
hero = {"x": 0, "hp": 3}
hero["x"] += 0.5
c = make_counter()
print(p.norm2(), hero, c(), list(countdown(3)), safe_div(1, 0))
```

### Classes

A class becomes a data struct plus a shared handle. Every field is typed by joining all
`self.x = ...` assignments in the program, and is an `Option` because Python allows reading
it before it is set (that raises `AttributeError`, as in CPython):

```rust
pub struct PointData {
    pub f_x: Option<i64>,
    pub f_y: Option<i64>,
    pub exc_msg: Option<rt::Str>,   // used when the class is an exception
    pub py: rt::PyCache,            // the Python wrapper while CPython holds this instance
}
pub struct Point(pub std::rc::Rc<std::cell::RefCell<PointData>>);

fn Point___init____Point_int_int(mut py_self: Point, mut x: i64, mut y: i64) -> PyResult<()> {
    py_self.0.borrow_mut().f_x = Some(x);      // (shortened)
    py_self.0.borrow_mut().f_y = Some(y);
    Ok(())
}
```

Methods are specialized like functions, with `self` as the first parameter
(`Point_norm2__Point`). The `Rc` gives Python's identity and aliasing: `q = p` shares the
instance. Dunders become trait implementations (`PyFmt` for `__repr__`/`__str__`, `PyEq`,
`PyOrd`, `Hash`, `Truthy`), so `print(p)`, `p == q`, `sorted(ps)` and `{p}` work natively.
A subclass gets its own struct holding the inherited fields. A value that can be one of
several classes is a generated enum, and a method call on it matches on the variant.

### Record dicts and `int | float`

A dict whose keys are string literals becomes a struct with a field per key. Keys added
later are optional fields, and a per-instance order list keeps printing in insertion
order:

```rust
/// `{"x": int | float, "hp": int}`: a dict with fixed keys, as a struct.
struct Rec_75b146f4367c(std::rc::Rc<std::cell::RefCell<Rec_75b146f4367c_d>>);
struct Rec_75b146f4367c_d { f0: rt::Num, f1: i64 }

hero = Rec_75b146f4367c::new(rt::Num::I(0_i64), 3_i64);
let __v = rt::num::add(hero.0.borrow().f0.clone(), rt::Num::F(0.5_f64))?;   // hero["x"] += 0.5
hero.0.borrow_mut().f0 = __v;
```

`hero["x"]` is an int at first and a float after `+= 0.5`, so it is an `rt::Num`. Each
operation on a `Num` picks the int or the float rule at runtime. A record used like a
general dict (a variable key, `.items()`, passing it to CPython) is compiled as a plain
`dict[str, V]` instead.

### Closures

Variables captured by a nested function are shared cells (`rt::Cell<T>`), so a write in one
function is seen by the other, as in Python. A closure value is an `rt::Env` holding those
cells, and calling it passes the environment:

```rust
fn make_counter__() -> PyResult<rt::Env<(rt::Cell<i64>,)>> {
    let n: rt::Cell<i64> = rt::cell();
    rt::cset(&n, 0_i64);
    return Ok(rt::Env((n.clone(),)));
}

fn inc_f5__fn5_int(__env: rt::Env<(rt::Cell<i64>,)>) -> PyResult<i64> {
    let n: rt::Cell<i64> = __env.0.0.clone();
    rt::cset(&n, rt::int::add(rt::cget(&n, "n")?, 1_i64)?);   // nonlocal n; n += 1
    return rt::cget(&n, "n");
}
```

A place that can hold several different functions (a dispatch table) is a generated enum
with one variant per function.

### Exceptions

Exceptions are `Err` values. A `try` body runs in a closure that returns
`PyResult<rt::Flow<T>>`, so `?` inside it stops at the `try`. `return`, `break` and
`continue` inside the body travel out as `Flow` values, which lets `finally` run before
they take effect:

```rust
fn safe_div__int_int(mut a: i64, mut b: i64) -> PyResult<f64> {
    let __r__t1 = (|| -> PyResult<rt::Flow<f64>> {
        return Ok(rt::Flow::Return(rt::int::truediv(a, b)?));
    })();
    let __r__t1 = match __r__t1 {
        Err(__e__t1) if rt::exc::matches(&__e__t1, "ZeroDivisionError") => (|| {
            return Ok(rt::Flow::Return(-1.0_f64));
        })(),
        other => other,
    };
    match __r__t1 {
        Ok(rt::Flow::Return(v)) => return Ok(v),
        Err(e) => return Err(e),
        ...
    }
}
```

`rt::exc::matches` follows CPython's builtin exception hierarchy (`except LookupError`
catches `KeyError`) and native exception classes. An exception that came from CPython keeps
its Python object, so `except json.JSONDecodeError` is an `isinstance` check and re-raising
keeps its identity.

### Generators

A generator body runs on its own small stack (a stackful coroutine, from the `corosensei`
crate). `yield` suspends that stack and hands the value to whoever is iterating; the next
`next()` or `send()` resumes it. The body itself is ordinary straight-line code:

```rust
fn countdown__int(n: i64) -> PyResult<rt::gen::Gen<i64, (), ()>> {
    Ok(rt::gen::Gen::new("countdown", move || countdown__int__body(n)))
}

fn countdown__int__body(mut n: i64) -> PyResult<()> {
    while n > 0_i64 {
        rt::gen::yield_::<i64, ()>(n)?;    // `?`: a throw() or close() arrives as an Err here
        n = rt::int::sub(n, 1_i64)?;
    }
    Ok(())
}
```

`Gen<Yield, Send, Return>` carries the three types of Python's `Generator[Y, S, R]`.
`throw()` and `close()` resume the body with an error, so `try`/`finally` inside it runs as
in CPython. Dropping an unfinished generator closes it. Generators still open at exit are
finalized in CPython's shutdown order (see the runtime table). A `for` loop owns its
generator, so `break` closes it at once.

### `async def`

Calling an `async def` creates an `rt::aio::Coro<T>`: the body, not yet started. Awaiting
it from another compiled coroutine runs it inline, as CPython does. When a coroutine
reaches CPython (`asyncio.run(main())`, `gather`, `create_task`), it is wrapped in a
four-line Python driver:

```python
async def _drive(step):
    done, v = step.send(None, None)
    while not done:
        try:
            r = await v
        except BaseException as e:
            done, v = step.send(None, e)
        else:
            done, v = step.send(r, None)
    return v
```

The body runs on its own stack, like a generator. When it awaits a Python awaitable
(`asyncio.sleep`, a task, a future), it suspends and passes the awaitable to the driver.
The driver awaits it in the event loop and resumes the body with the result or the
exception. Async generators use the same machinery, with two kinds of suspension: values
for the consumer, and awaitables for the event loop.

### Module globals and imported modules

A module variable read by a function, or any variable of an imported native module, is a
typed global cell. Reading it before its assignment has run raises `NameError`. An imported
`.py` module has an `init_<name>` function that runs its body once, at the first import.
At exit, module variables are released in the order CPython's shutdown uses, so a
generator's `finally` or an object's cleanup runs at the same point in the output.

### Native values in CPython

A native value passed to CPython is converted at the boundary:

* ints, floats, strings, bools and `None` become the Python equivalents;
* a native list, dict or set that can reach CPython is built as a Python object from the
  start (the escape rule), so both sides see the same object. A few read-only consumers
  (`json.dumps`, ...) receive a copy instead;
* a function becomes a Python callable that calls its all-`object` specialization;
* an instance becomes a `native_object`: Python can read and set fields, call methods,
  print, compare and hash it, and gets the same Python object back every time;
* generators become Python iterators, coroutines become awaitables.

### Rust and C imports

A `.rs` file is compiled as a module of the generated crate, so a call is an ordinary Rust
call. Arguments are converted with checks: a list passed as `&mut [f64]` is copied in and
written back after the call, and a Python function passed as `impl Fn(i64) -> i64` becomes
a closure that calls back into it. `pub struct` types are handles that Python code holds
and calls methods on. A `.c` file is compiled by `build.rs` with the `cc` crate and called
through `extern "C"` declarations generated from its prototypes. Pointer-and-length pairs
take lists, a `typedef struct` is created as `S(field, ...)` with its fields as attributes, and function pointers take Python
functions through generated trampolines. [imports.md](imports.md) has the type tables.

## 7. Building

The driver writes a small Cargo project to `.pyfast/<name>/`:

```
.pyfast/demo/
├── Cargo.toml              # depends on the runtime crate (embedded in the binary, extracted to
│                           # ~/.cache/pyfast/runtime/<hash>/), + "python" feature, + cc if C is used
├── build.rs                # only if C files are imported
├── src/main.rs             # the generated code above
├── pyfast-report.json      # what explain shows, as JSON
├── pyfast.sourcemap.json   # Rust function → Python file:line
└── demo                    # the executable, copied here after the build
```

It then runs `cargo build` with:

* a **shared target directory**, `~/.cache/pyfast/target` (override it with
  `PYFAST_TARGET_DIR`), so the runtime and PyO3 are compiled once for all your projects;
* a crate name hashed from the output directory (`pyfast_demo_2b2066bd`), so two `main.py`
  files in different folders don't overwrite each other;
* `PYO3_PYTHON` set to the interpreter you build with, so the binary links that Python's
  libpython.

The `python` feature, and with it PyO3 and libpython, is only enabled when the program has
CPython boundaries. A fully native program is a plain Rust binary.

Cargo's own incremental build is the cache: an unchanged program rebuilds in well under a second,
and the runtime and PyO3 are compiled once per toolchain and Python version.

**`pyfast-report.json`** holds everything `explain` prints: `mode`, `parser` (CPython
version, AST protocol), `imports`, `externs` (Rust/C functions and their accepted or
rejected signatures), `generic_signatures`, `specializations` (each with `spec_id`,
`function`, `signature`, `rust_name`, `tier`, `effects`, `recursive` and the source `span`),
`boundaries` (code, location, what and why), `diagnostics`, `counts` (native/bridge),
`unreachable_functions`, `int_semantics` and `runtime_requirements` (whether CPython and a
C compiler are needed).

**`pyfast.sourcemap.json`** maps each generated Rust function to its Python file, line and
column, so a Rust compile error or a profiler frame can be traced back to the source.

---

## The runtime: Python semantics in Rust

[crates/runtime](../crates/runtime/src/) is linked into every program. It's where Python
behaviour is reproduced:

| Module | What it guarantees |
|---|---|
| `int.rs` | Checked `+ - * **` and shifts (overflow raises `OverflowError`, not wraparound). Python's floor division and modulo signs. Correctly rounded `int / int`. Exact int-vs-float comparison |
| `float.rs` | `repr` identical to CPython (`1e+16`, `1e-05`, `-0.0`, `inf`), and Python's float `//`, `%` and `**` edge cases |
| `string.rs`, `conv.rs` | `str()`, `int("..")`, `float("..")`, `repr`, `in`, repetition, with Python's error messages |
| `exc.rs` | `Exc { kind, msg }`. An uncaught one prints a short `Traceback` header and `Kind: message`, then exits with status 1 |
| `io.rs` | Buffered `print`, flushed at exit and before every CPython call so output stays in order; `run_main`, and the finalization of module variables at exit (CPython's shutdown order) |
| `coll.rs` | `List`, `Dict` (insertion-ordered), `Set`, tuples: shared `Rc<RefCell<..>>` so aliasing matches Python |
| `num.rs` | `Num`: a value that is an int on some paths and a float on others, with exact int or float semantics per operation |
| `stackful.rs`, `gen.rs` | Generators run on their own stack (`corosensei`): `yield` suspends it; `send`/`throw`/`close`; an abandoned generator is closed |
| `aio.rs`, `agen.rs` | `async def` bodies and async generators, driven by asyncio through a small Python driver; `await` of a Python awaitable suspends the body's stack |
| `math.rs` | The `math` module with CPython's errors |
| `ext.rs` | Checked conversions for Rust/C calls (`int_to`, `cstring`, `from_cstr`, `rust_err`, callbacks) |
| `py.rs` | The CPython bridge (PyO3), described below |

### The CPython bridge

`rt::py` wraps PyO3:

* `init` starts the interpreter once, sets `sys.path` to the paths captured at build time and
  makes stdout unbuffered, so native and Python output don't interleave out of order.
* `Obj` is an owned Python object reference. `getattr`, `call`, `binop`, `compare`, `iter`,
  `list`, `dict` and the others perform one operation each. The program runs attached to
  the interpreter (`rt::py::attached` around the module body), so an operation only bumps
  PyO3's attach counter and a dropped object is released at once. String constants are
  interned once per call site (`rt::pystr!`).
* State shared by every thread that runs compiled code (module globals, stdout, the
  exception-class registry) lives in `rt::Shared` cells, sound because compiled code only
  runs while holding the GIL; native functions and objects can be called from CPython
  threads.
* `Slot` is a lazily filled static for each CPython import. `import` fills it, and `get`
  reads it, raising `NameError` if it was never imported.
* Python exceptions are converted to `Exc` with their full qualified type name
  (`json.decoder.JSONDecodeError`), so error output matches CPython's last traceback line.
* Boxing and unboxing between native values and objects (`int(i64)`, `to_i64(&obj)`,
  `unbox_annotated::<i64>`) is checked. A Python int that doesn't fit `i64` raises
  `OverflowError` rather than being truncated.

## Why the output matches CPython

The test suite compiles each program in [tests/differential](../tests/differential/), runs
it, and compares its stdout, exit code and final error line with `python3` running the same
file. Snapshot tests pin the generated Rust ([tests/golden](../tests/golden/)) and the
diagnostics ([tests/diagnostics](../tests/diagnostics/)). [samples/run_all.sh](../samples/run_all.sh)
does the same for every sample program.

Known differences, such as ints beyond 64 bits, are listed in [subset.md](subset.md).

## Seeing all of this yourself

```console
pyfast explain main.py                 # imports, generic schemes, specializations, boundaries
pyfast explain main.py --format json   # the same, machine-readable
pyfast emit main.py --out /tmp/x       # write the Cargo project without building
pyfast check main.py --mode native-only   # list every place the program would need CPython
```
