# A tour of what compiles

Small, ordinary Python programs, one idea each, numbered in reading order. Every one is
checked by `run_all.sh`: the compiled executable must print the same stdout, exit with the
same status and end with the same exception line as CPython. The Rust and C programs can't
run under CPython, so they are checked against `expected.txt` / `expected.err` instead.

From this folder:

```console
bash run_all.sh                    # build and check every program (debug builds)
bash run_all.sh --release 08       # one program, optimized
pyrock explain 06_generics.py      # what was inferred for an unannotated function
pyrock run 08_mandelbrot.py
pyrock bench 04_collatz.py --runs 3
```

| Program | Shows | Runtime |
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
| [22_native_sieve/](22_native_sieve/) | A prime sieve kept fully native (`--mode native-only`), with `bench` | native only |
| [23_native_mandelbrot/](23_native_mandelbrot/) | The ASCII Mandelbrot set again, checked with `--mode native-only` | native only |

"native only" means the binary doesn't link libpython at all. In `pyrock explain`, the
"CPython boundaries" section lists every place a program crosses into Python, with the reason.
