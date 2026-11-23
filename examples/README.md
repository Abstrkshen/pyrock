# Examples

Programs that show what pyrock does with real code, grouped by what you want to see.
Each example has a README whose commands you can run as written, from its folder. CI
runs them all except the ones marked as needing something CI doesn't have.

Install pyrock first (see the [main README](../README.md#installation)). To try pyrock
without installing anything, use the [playground](https://abstrkshen.github.io/pyrock/play.html).

## [tour/](tour/): what compiles, one idea at a time

23 small numbered programs: from `hello` through generics, annotations, strings, errors,
local modules, Rust and C modules, to all of them together. `bash tour/run_all.sh` checks
each one against CPython.

## [interop/](interop/): Rust, C and Python modules

| Example | What it shows | Needs |
|---|---|---|
| [py-modules](interop/py-modules/) | Two imported `.py` modules compiled into the same binary | |
| [rust-vecmath](interop/rust-vecmath/) | Calling functions in a `.rs` file without bindings | |
| [rust-http](interop/rust-http/) | A Rust HTTP server started from Python, queried with `urllib` | |
| [import-c](interop/import-c/) | A `.c` file compiled in, and a C++ shared library through `ctypes` | `g++` |
| [hybrid-json](interop/hybrid-json/) | A CPython module (`json`) whose results flow into native code | |
| [imports](interop/imports/) | Every import kind in one program | a C compiler |

## [packages/](packages/): pip packages through CPython

| Example | What it shows | Needs |
|---|---|---|
| [huggingface](packages/huggingface/) | A `huggingface_hub` model card | the virtualenv below |
| [langchain](packages/langchain/) | A LangChain prompt and chain, with native callbacks | the virtualenv below |

## [apps/](apps/): larger programs

| Example | What it shows | Needs |
|---|---|---|
| [backend](apps/backend/) | A JSON API on asyncio: native store, routing and validation; sockets and `json` through CPython | |
| [ai](apps/ai/) | A BPE tokenizer in Python, a bigram model in Rust, sampling in C; no CPython at all | |
| [gui](apps/gui/) | 300 colliding balls: native physics (16x CPython) drawn with raylib | `pip install raylib` (see its README) |
| [ollama-agents](apps/ollama-agents/) | An AI-agent HTTP server: native Python logic, Rust HTTP and JSON, no CPython at runtime | [Ollama](https://ollama.com) with `qwen3.5:4b` |

`bash apps/run_all.sh` checks backend, ai and gui against their expected output.

## [graphics/](graphics/): windows, games and vision

| Example | What it shows | Needs |
|---|---|---|
| [raylib](graphics/raylib/) | A window drawn by the C raylib library | `bash build.sh` |
| [raylib-breakout](graphics/raylib-breakout/) | A platformer with SVG sprites; game logic native, drawing in C | `bash build.sh`, librsvg, cairo |
| [opencv-yolo](graphics/opencv-yolo/) | Live YOLO detection, segmentation and pose from a webcam: OpenCV, ONNX Runtime and raylib in C/C++ | `bash build.sh` (downloads and builds), a camera |

## [playground/](playground/): the browser playground's programs

The programs in the [playground](https://abstrkshen.github.io/pyrock/play.html): a ray
tracer, a Lisp interpreter, maze search, flocking birds, chaotic pendulums, an L-system
plant, and smaller ones. Most draw by printing SVG, HTML or PPM images, which the
playground renders and animates. All compile with `--mode native-only`, and the test suite
checks each against CPython.

## Setup

The `packages` examples import Python packages, which pyrock loads through CPython. They
use one virtualenv at `examples/.venv`:

```sh
cd examples
python3 -m venv .venv                       # or: uv venv .venv
.venv/bin/pip install -r requirements.txt   # or: uv pip install --python .venv/bin/python -r requirements.txt
```

The `graphics` examples load native libraries from their `lib/` folders; each has a
`build.sh` that downloads or builds them (nothing binary is committed).
`graphics/fetch-raylib.sh` downloads the official raylib 5.5 Linux x86-64 release. These
examples are written for Linux x86-64.

Third-party files kept here: `graphics/raylib/raylib.h` (raylib 5.5, zlib licence) and
`graphics/opencv-yolo/include/onnxruntime_*.h` (ONNX Runtime 1.32 C API, MIT licence).

`scripts/readme_check.sh` in the repository root runs every example's README commands; in
CI it skips the examples marked `skip-ci`, which need a display, a camera, Ollama or the
virtualenv.
