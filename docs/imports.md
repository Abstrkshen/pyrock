# Imports

pyfast programs import Python, Rust and C with the same plain `import` statement. You don't
write any binding code or configuration. The file you name decides what happens.

```python
import json        # nothing local called json → CPython's json module
import geometry    # geometry.py next to this file → compiled to Rust with your program
import vecmath     # vecmath.rs next to this file  → linked into the binary
import fastc       # fastc.c (+ fastc.h)           → compiled with the C compiler and linked
```

The full working version is in [examples/imports/](../examples/imports/). Run it with
`pyfast run examples/imports/main.py`.

## How a name is resolved

For `import x` or `from x import f`, pyfast looks in the **directory of the file containing the
import**, and stops at the first match:

1. `x.py`: a Python module, compiled natively
2. `x.rs`: a Rust module
3. `x.c`: a C module (with `x.h` if present)
4. otherwise, a **CPython import** at runtime

Run `pyfast explain main.py` to see the choice for every import:

```
Imports
  json      -> cpython
  geometry  -> python (native)  /path/to/geometry.py
  vecmath   -> rust  /path/to/vecmath.rs  (3 function(s))
      vecmath.dot(f64, f64, f64, f64) -> f64
      vecmath.greet(&str) -> String
      vecmath.checked_div(i64, i64) -> i64  [Result: errors raise RuntimeError]
```

Supported forms: `import x`, `import x as y`, `from x import f`, `from x import f as g`.
A dotted name (`import os.path`, `import xml.etree.ElementTree`) always goes to CPython.

## Importing Python files

```python
# geometry.py
def circle_area(r):
    return 3.141592653589793 * r * r
```

```python
# main.py
import geometry
from geometry import circle_area

print(geometry.circle_area(2.0), circle_area(1.0))
```

* The imported file goes through the same compiler as the main file, and its functions become
  ordinary Rust functions in the same binary. A call such as `geometry.circle_area(2.0)` is a
  direct Rust call. It doesn't look anything up at runtime.
* Its functions are specialized for the types *your* program passes. `circle_area(2.0)`
  produces `geometry_circle_area__float`. A function nobody calls isn't compiled at all.
* Its module body runs **once**, at the first import, as in Python.
* A module can import other modules, resolved relative to its own directory.

Limits of native Python modules:

* You can call a module's **functions**, but you can't read its module-level variables
  (`geometry.PI`), because module globals aren't compiled yet. Wrap the value in a function, or
  make it a local constant.
* Circular imports between native modules are rejected (P145).
* Relative imports (`from . import x`) and `from x import *` aren't supported.
* Imports must be at module level, not inside a function.

**Fallback.** If an imported `.py` uses something pyfast can't compile, such as a `class`,
`try`, a comprehension or a lambda, hybrid mode imports that file through CPython instead and
warns:

```
P220 warning: `shapes.py` cannot be compiled natively; importing it through CPython
    note: first blocker: ...
```

The program still works. Calls into that module become CPython calls, listed in `explain`.
`--mode native-only` makes this an error.

## Importing Rust files

Put a `.rs` file next to your Python file. Every `pub fn` with supported types is callable:

```rust
// vecmath.rs
pub fn dot(ax: f64, ay: f64, bx: f64, by: f64) -> f64 { ax * bx + ay * by }

pub fn greet(name: &str) -> String { format!("hello, {name}, from Rust") }

pub fn checked_div(a: i64, b: i64) -> Result<i64, String> {
    if b == 0 { Err("division by zero in Rust".to_string()) } else { Ok(a / b) }
}

fn private_helper() {}   // not pub → not visible from Python
```

```python
import vecmath
print(vecmath.dot(1.5, 2.0, 3.0, 4.0))   # 11.0
print(vecmath.greet("pyfast"))           # hello, pyfast, from Rust
vecmath.checked_div(1, 0)                # RuntimeError: division by zero in Rust
```

| Rust type | Python side |
|---|---|
| `i8`…`i64`, `u8`…`u64`, `isize`, `usize` | `int`. Out-of-range values raise `OverflowError` |
| `f32`, `f64` | `float` (an `int` argument is converted) |
| `bool` | `bool` |
| `&str`, `String` | `str` |
| `()` | `None` |
| `Result<T, E>` (return only) | `T`. `Err(e)` raises `RuntimeError(str(e))`. `E` must implement `Display` |

The file is compiled as a module of the generated crate (`#[path = "..."] mod rs_vecmath;`),
so it can contain any private helpers, structs, `use std::...` and so on. Only the `pub fn`
signatures have to stick to the table. Functions with generics, references to other types,
structs or slices in their signature are skipped. `explain` lists them with the reason, and
calling one gives P150. The `.rs` file can't depend on external crates yet.

## Importing C files

Put `x.c` (and optionally `x.h`) next to your Python file:

```c
// fastc.h
#include <stdint.h>
int64_t gcd(int64_t a, int64_t b);
double scale(double x, int k);
const char *version(void);
```

```python
import fastc
print(fastc.gcd(1071, 462))   # 21
print(fastc.scale(2.5, 4))    # 10.0
print(fastc.version())        # a Python str
```

* Signatures are read from `x.h` if it exists, otherwise from the non-`static` function
  definitions in `x.c`.
* The `.c` file is compiled with the system C compiler (through the `cc` crate) and linked
  in, along with `libm`.
* Supported types are the integer types (`int`, `long`, `int64_t`, `size_t`, `unsigned`, ...),
  `float`/`double`, `bool`, `const char *` (to and from `str`) and `void`.
* Checks at the boundary: an int that doesn't fit the C type raises `OverflowError`, and a
  string containing a NUL byte raises `ValueError`.
* Pointers other than `const char *`, structs, variadic functions and `static` functions
  aren't callable. `explain` lists them as skipped.

C strings returned from C are copied into a Python `str` immediately. pyfast doesn't free
them, so return static or long-lived strings.

## Importing anything else (CPython)

If nothing local matches, the import happens in an **embedded CPython interpreter** at
runtime. This covers the standard library, pure-Python packages in site-packages, and
C-extension packages such as numpy:

```python
import json
from collections import Counter
import numpy as np

print(json.dumps({"a": 1}))
print(np.arange(5).sum())
```

* The `sys.path` of the interpreter that ran the build is embedded in the binary. If you
  build inside a virtualenv (or pass `--python path/to/venv/bin/python`), the binary finds
  that venv's packages.
* Values from these modules are Python objects. Everything you do with them (method calls,
  indexing, operators) goes through CPython, with exact Python semantics.
* You can pass these values straight to native, Rust or C functions. They're unboxed with a
  check: `fastc.gcd(json.loads("12"), 18)` works, and passing a str where an `int` is needed
  raises `TypeError`.
* Every such use is a **boundary** (codes P200/P210/...), listed under "CPython boundaries"
  in `explain`. With `--mode native-only` each one is a compile error, which you can use to
  make sure a program never touches Python.
* `typing`, `typing_extensions` and `__future__` imports used only in annotations are
  removed, so they don't pull in CPython.

A program with no CPython imports and no boundaries doesn't link libpython at all.

## Quick reference

| You write | Next to the file | Result |
|---|---|---|
| `import m` | `m.py` | Native Rust, specialized per call site types |
| `import m` | `m.py` that uses classes, `try`, ... | CPython import + P220 warning (hybrid) |
| `import m` | `m.rs` | Rust `pub fn`s called directly |
| `import m` | `m.c` (+ `m.h`) | C functions called directly through FFI |
| `import m` | nothing | CPython runtime import |
| `import a.b` | anything | CPython runtime import |
| `from . import m` | anything | Rejected (relative import) |
