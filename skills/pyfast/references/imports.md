# pyfast imports reference

Resolution for `import x`, `import x as y`, `from x import f` and `from x import f as g`
happens in the **directory of the file that contains the import**. The first match wins:

1. `x.py`: native Python unit
2. `x.rs`: Rust module
3. `x.c`, with signatures from `x.h` if it exists: C module
4. otherwise a CPython runtime import

`pyfast explain file.py` prints an `Imports` section with the decision for each one, plus the
callable and skipped Rust/C signatures.

Rejected: `from . import x` (relative), `from x import *`, imports inside a function or block.
A dotted name (`import a.b`) never resolves locally. It is always a CPython import.

## Native Python modules (`x.py`)

```python
# geometry.py
def circle_area(r):
    return 3.141592653589793 * r * r

# main.py
import geometry
from geometry import circle_area as area
print(geometry.circle_area(2.0), area(1.0))
```

* The module is compiled into the same binary. `geometry.circle_area(2.0)` becomes a direct
  call to `geometry_circle_area__float`.
* Only functions reachable from the entry program are compiled, each specialized for the
  argument types actually used.
* The module body runs once, at the first import (`init_<name>` guarded by an `AtomicBool`).
* Nested imports resolve relative to the importing module's own directory.
* Only **functions** are accessible: `geometry.CONST` and `from geometry import CONST` are
  rejected (P100/P140). Expose constants through a function.
* Circular native imports: P145.
* **Fallback:** if the module has any compile error in hybrid mode (e.g. it defines a
  `class` or uses `try`), it's imported through CPython instead, with warning
  `P220 ... cannot be compiled natively; importing it through CPython`. The note shows the
  first blocker. Calls into it are then CPython boundaries. In native-only mode the errors are
  reported instead.

## Rust modules (`x.rs`)

```rust
// vecmath.rs
pub fn dot(ax: f64, ay: f64, bx: f64, by: f64) -> f64 { ax * bx + ay * by }
pub fn greet(name: &str) -> String { format!("hello, {name}") }
pub fn checked_div(a: i64, b: i64) -> Result<i64, String> {
    if b == 0 { Err("division by zero".into()) } else { Ok(a / b) }
}
```

* Included as `#[path = "/abs/x.rs"] mod rs_x;` in the generated crate, so it's compiled with
  the program and can contain private helpers, structs and `use std::...`.
* External crates: add a `// pyfast: dep NAME = "VERSION"` comment line (any Cargo dependency
  spec after the `=`, e.g. `// pyfast: dep serde = { version = "1", features = ["derive"] }`).
  It is copied into the generated `Cargo.toml`.
* Parsed with `syn`. Only `pub fn` at the top level of the file are exposed.

| Rust signature type | Python value |
|---|---|
| `i8 i16 i32 i64 isize u8 u16 u32 u64 usize` | `int`. Range-checked, out of range → `OverflowError` |
| `f32`, `f64` | `float` (int and bool arguments accepted) |
| `bool` | `bool` |
| `&str` | `str` (argument) |
| `String` | `str` (argument or return) |
| `()` / no return | `None` |
| `Result<T, E>` return | `T`. `Err(e)` raises `RuntimeError(e.to_string())` |
| `Vec<T>`, `&[T]`, `&Vec<T>` | any iterable of `T` (list, tuple, `array.array`, numpy array, ...); a returned `Vec<T>` is a `list` |
| `&mut [T]`, `&mut Vec<T>` | a list (or any mutable sequence, e.g. `array.array`); the changes are written back after the call |
| `Vec<u8>` / `&[u8]` | `bytes`, `bytearray` or a list of ints; a returned `Vec<u8>` is `bytes` |
| `Option<T>` | `T` or `None` |
| `(A, B, ...)` | `tuple` |
| `HashMap<K, V>`, `BTreeMap<K, V>` | `dict` |
| `impl Fn(A) -> R`, `&dyn Fn(A) -> R` | a function, lambda or closure; it's compiled natively for the argument types, and an exception inside it propagates out of the Rust call |
| `pub struct S` (with `#[derive(Clone)]`) | a handle object: `S(...)`/`S.new(...)` (an `impl` fn named `new`), `pub` fields read and assigned as attributes, `impl` methods taking `&self`/`&mut self` called as methods |

Struct handles are shared by reference (like Python objects), print as `S { .. }` (Rust
`Debug`) and must stay native: passing one to CPython is P231. Associated functions
are called as `S.name(...)`.

Skipped (listed in `explain` with a reason, calling one gives P150): generic functions,
`async`, `unsafe`, `&mut` of scalars, references to unsupported types. Wrong argument types
or count from Python: P303.

## C modules (`x.c` + optional `x.h`)

```c
// fastc.h
#include <stdint.h>
int64_t gcd(int64_t a, int64_t b);
double scale(double x, int k);
const char *version(void);
```

* Signatures come from prototypes in `x.h` if present, otherwise from non-`static` function
  definitions in `x.c`. Comments are stripped. Macros aren't expanded, so keep prototypes
  literal.
* The generated `build.rs` compiles `x.c` with the `cc` crate (the header directory is added
  to the include path) and links `libm`.
* Libraries: `// pyfast: link NAME` adds `-lNAME`; `// pyfast: pkg-config NAME` adds the
  package's compile and link flags (e.g. `// pyfast: pkg-config raylib`).

| C type | Python value |
|---|---|
| `int`, `long`, `long long`, `short`, `char`, `signed`/`unsigned` variants, `int8_t`…`uint64_t`, `size_t`, `ssize_t` | `int`, range-checked → `OverflowError` |
| `float`, `double` | `float` |
| `bool` / `_Bool` | `bool` |
| `const char *` / `char *` argument | `str`. NUL inside → `ValueError` |
| `const char *` return | `str` (copied, never freed, so return static or long-lived strings) |
| `void` | `None` |
| `T *data, int n` / `const T *data, size_t n` (pointer followed by an integer length) | one argument: a sequence of `T`; a non-`const` pointer writes the changes back |
| `typedef struct { ... } S;` by value | `S(field, ...)` creates it; fields are attributes; a returned struct is an `S` |
| `R (*f)(A, ...)` | a function, lambda or closure (called through a generated `extern "C"` trampoline; it must not be kept after the call) |

Skipped: other pointer types, unions, variadic (`...`), `static`.

## CPython imports

* Anything with no local file: stdlib (`json`, `math`, `collections`, ...), pure-Python
  packages and C extensions (`numpy`). Each import is a P200 boundary.
* `sys.path` of the build interpreter is baked into the binary. Build with a venv's python
  (`--python .venv/bin/python`, or with the venv active) to use its packages.
* Values from CPython are `Obj`. Operations on them go through CPython, with exact Python
  semantics, aliasing and mutation.
* Passing an `Obj` to a native, Rust or C parameter unboxes it with a runtime check
  (`TypeError` on a wrong type, `OverflowError` on a too-large int).
* `typing`, `typing_extensions` and `__future__` imports that are only used in annotations
  are dropped and never touch CPython.
* A program with no CPython boundaries doesn't link libpython at all.

## Example layout

```
project/
├── main.py        # import geometry, vecmath, fastc, json
├── geometry.py    # native
├── vecmath.rs     # Rust
├── fastc.c        # C
└── fastc.h
```

```sh
pyfast explain project/main.py
pyfast run project/main.py
```

Working examples in the repo: `examples/imports/`, `samples/18_local_modules/`,
`samples/19_rust_module/`, `samples/20_c_module/`, `samples/21_everything/`,
`tests/interop/rust_c/`, `tests/interop/rich/` (slices, structs, callbacks, crates, C arrays and
structs), `tests/interop/buffers/` (`array.array`, `bytes`).
