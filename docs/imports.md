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

Supported forms: `import x`, `import x as y`, `from x import f`, `from x import f as g`,
dotted and relative imports of packages (below), and imports inside functions and blocks.
A dotted name that isn't a package next to the program (`import os.path`,
`import xml.etree.ElementTree`) goes to CPython.

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

* Its functions, classes and module-level variables are all usable: `geometry.Point(1, 2)`,
  `from geometry import Point`, `geometry.PI`, `from geometry import PI` (copied at import,
  as Python does), `geometry.PI = 3.0` (a typed global cell).
* Circular imports between native modules are rejected (P145), except a package's
  `__init__.py` importing its own submodules (as CPython allows).
* `from x import *` isn't supported.
* Imports can be inside functions and blocks. A CPython import there assigns a variable
  (`try: import numpy as np` / `except ImportError: np = None` works). A native module
  imported inside a function binds its names for that function, with direct calls.

## Packages

A directory with an `__init__.py` next to the program (or next to the entry file, which is
`sys.path[0]` in CPython) is a package, and its modules compile natively too:

```
main.py
app/
  __init__.py          # from . import models
  models.py
  services/
    __init__.py
    billing.py         # from ..models import Item; from . import rules
    rules.py
```

```python
# main.py
import app.services.billing
from app import models
from app.services.billing import total
import app.services.rules as rules
```

* `import a.b.c` runs `a/__init__.py`, `a/b/__init__.py` and `a/b/c.py` once each, in
  that order, and binds `a`; `a.b.c.f()` is then a direct call. `import a.b as x` binds the
  module itself.
* `from a import b` imports the submodule `b` when `a` has no function, class or variable
  of that name.
* Relative imports (`from . import x`, `from .x import f`, `from ..models import Item`)
  resolve against the importing module's package. A relative import of a module that
  doesn't exist raises `ModuleNotFoundError` where it runs, so `try: from ._speedups import
  f` / `except ImportError:` works.
* Names a module imported are importable from it (`from .compat import urlparse`, where
  `compat` did `from urllib.parse import urlparse`), and callable as `compat.urlparse(...)`.
* A package module that can't be compiled is imported through CPython (P220), as for a
  single `.py` file.

**Fallback.** If an imported `.py` uses something pyfast can't compile, such as a class with
a metaclass or a CPython base class, hybrid mode imports that file through CPython instead and
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
| `Vec<T>`, `&[T]`, `&Vec<T>` | any iterable of `T` (list, tuple, `array.array`, numpy array, ...); a returned `Vec<T>` is a `list` |
| `&mut [T]`, `&mut Vec<T>` | a list or other mutable sequence; changes are written back after the call |
| `Vec<u8>`, `&[u8]` | `bytes`, `bytearray` or a list of ints; a returned `Vec<u8>` is `bytes` |
| `Option<T>` | `T` or `None` |
| `(A, B, ...)` | `tuple` |
| `HashMap<K, V>`, `BTreeMap<K, V>` | `dict` |
| `impl Fn(A) -> R`, `&dyn Fn(A) -> R` | a function, lambda or closure, compiled natively for those argument types; an exception inside it propagates out of the call |
| `pub struct S` (`#[derive(Clone)]`) | a handle: `S.new(...)` (an `impl` fn `new`), `pub` fields as attributes, `&self`/`&mut self` methods as methods |

```rust
// geo.rs
// pyfast: dep itoa = "1"
pub fn scale_all(xs: &mut [f64], k: f64) { for x in xs.iter_mut() { *x *= k; } }
pub fn apply(xs: Vec<i64>, f: impl Fn(i64) -> i64) -> Vec<i64> { xs.into_iter().map(f).collect() }

#[derive(Debug, Clone)]
pub struct Counter { pub name: String, pub count: i64 }
impl Counter {
    pub fn new(name: &str) -> Counter { Counter { name: name.to_string(), count: 0 } }
    pub fn incr(&mut self, by: i64) { self.count += by; }
}
```

```python
import geo
xs = [1.0, 2.0]
geo.scale_all(xs, 10.0)                 # xs is now [10.0, 20.0]
print(geo.apply([1, 2], lambda v: v * 3))
c = geo.Counter.new("hits")
c.incr(2)
print(c.count)
```

The file is compiled as a module of the generated crate (`#[path = "..."] mod rs_geo;`), so
it can contain private helpers, structs and `use` lines. External crates are added with a
`// pyfast: dep NAME = "VERSION"` comment (any Cargo dependency spec after the `=`).
Struct handles are shared by reference and must stay native: passing one to CPython is
P231. Generic, `async` and `unsafe` functions are skipped; `explain` lists them with the
reason, and calling one gives P150.

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
* Libraries: `// pyfast: link NAME` adds `-lNAME`; `// pyfast: pkg-config NAME` adds the
  package's compile and link flags.

| C type | Python side |
|---|---|
| integer types (`int`, `long`, `int64_t`, `size_t`, `unsigned`, `char`, ...) | `int`, range-checked (`OverflowError`) |
| `float`, `double` | `float` |
| `bool` / `_Bool` | `bool` |
| `const char *` / `char *` | `str` (a NUL inside raises `ValueError`) |
| `void` | `None` |
| `T *data, int n` / `const T *data, size_t n` (pointer followed by a length) | one argument: a sequence of `T`; a non-`const` pointer writes the changes back |
| `typedef struct { ... } S;` by value | `S(field, ...)` creates it; fields are attributes; a returned struct is an `S` |
| `R (*f)(A, ...)` | a function, lambda or closure (through a generated `extern "C"` trampoline; it must not be kept after the call) |

Unions, variadic functions (`...`) and `static` functions aren't callable; `explain` lists
them as skipped.

C strings returned from C are copied into a Python `str` immediately. pyfast doesn't free
them, so return static or long-lived strings.

## Importing anything else (CPython)

`import math` is compiled natively when every use is a call or constant on native numbers
(`math.sqrt(x)`, `math.pi`); then no CPython is needed for it.

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
| `import a.b` | `a/__init__.py`, `a/b.py` | Native package modules |
| `import a.b` | nothing | CPython runtime import |
| `from . import m` / `from .m import f` | in a package | Native, relative to the package |
