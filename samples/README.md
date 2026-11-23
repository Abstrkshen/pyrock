# Sample programs

Ordinary Python programs that pyrock compiles to native executables. Every one is checked
by `run_all.sh`: the compiled binary must produce the same stdout, exit status and final
exception line as CPython. The Rust/C samples can't run under CPython, so they are checked
against `expected.txt` / `expected.err`.

```console
samples/run_all.sh                 # build + check everything (debug builds)
samples/run_all.sh --release 08    # one sample, optimised
target/debug/pyrock explain samples/06_generics.py
target/debug/pyrock run samples/08_mandelbrot.py
target/debug/pyrock bench samples/04_collatz.py --runs 3
```

| Sample | Shows | Runtime |
|---|---|---|
| [01_hello.py](01_hello.py) | Smallest program, f-strings | native only |
| [02_fizzbuzz.py](02_fizzbuzz.py) | `if/elif/else`, `range`, string building | native only |
| [03_primes.py](03_primes.py) | Integer loops, early `return`, prime counting | native only |
| [04_collatz.py](04_collatz.py) | Tight integer loops, floor division (good for `bench`) | native only |
| [05_fibonacci.py](05_fibonacci.py) | Recursion (with the recursion limit) vs iteration | native only |
| [06_generics.py](06_generics.py) | One unannotated function, many specializations | native only |
| [07_annotations.py](07_annotations.py) | Full and partial annotations, unboxing `json` results | CPython for `json` |
| [08_mandelbrot.py](08_mandelbrot.py) | Nested float loops, ASCII output | native only |
| [09_numerics.py](09_numerics.py) | Newton's method, integration, series, float edge cases | native only |
| [10_number_theory.py](10_number_theory.py) | gcd/lcm/modpow, Python floor-div/mod signs, shifts | native only |
| [11_strings.py](11_strings.py) | Native str ops plus str methods, slicing, `ord`/`chr`, format specs | CPython for methods |
| [12_json_report.py](12_json_report.py) | Native results into dicts/lists and `json` | CPython |
| [13_word_count.py](13_word_count.py) | `collections.Counter`, `sorted`, iteration | CPython |
| [14_aliasing.py](14_aliasing.py) | List/dict aliasing and in-place `+=`, exactly as Python | CPython objects |
| [15_math_stdlib.py](15_math_stdlib.py) | A C-extension module (`math`), `from ... import` | CPython |
| [16_boxing.py](16_boxing.py) | Variables/returns that change type are boxed | CPython objects |
| [17_errors.py](17_errors.py) | Uncaught `ZeroDivisionError`: same message, exit status 1 | native only |
| [18_local_modules/](18_local_modules/) | Importing your own `.py` files (compiled natively, run once) | native only |
| [19_rust_module/](19_rust_module/) | `import fastmath` → `fastmath.rs`; `Result` errors become `RuntimeError` | native only |
| [20_c_module/](20_c_module/) | `import clib` → `clib.c` + `clib.h`; `OverflowError` for a too-large C `int` | native only |
| [21_everything/](21_everything/) | Python + Rust + C + CPython packages in one program | CPython for stdlib |

"native only" means the binary doesn't link libpython at all. In `pyrock explain`, the
"CPython boundaries" section lists every place a program crosses into Python, with the reason.
