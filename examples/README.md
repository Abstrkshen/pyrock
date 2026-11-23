# Examples

Small programs that show what pyrock does with real code: plain Python compiled to native
code, Rust and C modules imported directly, and Python packages used through the embedded
CPython. Each folder has a README whose commands you can run as written, from that folder.

| Example | What it shows | Needs |
|---|---|---|
| [hello](hello/) | The smallest program | |
| [native-sieve](native-sieve/) | A prime sieve; the whole program is native (`--mode native-only`) | |
| [native-mandelbrot](native-mandelbrot/) | An ASCII Mandelbrot set, fully native | |
| [py-modules](py-modules/) | Two imported `.py` modules compiled into the same binary | |
| [rust-vecmath](rust-vecmath/) | Calling functions in a `.rs` file without bindings | |
| [rust-http](rust-http/) | A Rust HTTP server started from Python, queried with `urllib` | |
| [import-c](import-c/) | A `.c` file compiled in, and a C++ shared library through `ctypes` | `g++` |
| [hybrid-json](hybrid-json/) | A CPython module (`json`) whose results flow into native code | |
| [huggingface](huggingface/) | A `huggingface_hub` model card, through CPython | the virtualenv below |
| [langchain](langchain/) | A LangChain prompt and chain, with native callbacks | the virtualenv below |
| [ollama-agents](ollama-agents/) | An AI-agent HTTP server: native Python logic, Rust HTTP and JSON, no CPython at runtime | [Ollama](https://ollama.com) with `qwen3.5:4b` |
| [raylib](raylib/) | A window drawn by the C raylib library | `bash build.sh` |
| [raylib-breakout](raylib-breakout/) | A platformer with SVG sprites; game logic native, drawing in C | `bash build.sh`, librsvg, cairo |
| [opencv-yolo](opencv-yolo/) | Live YOLO detection, segmentation and pose: OpenCV, ONNX Runtime and raylib in C/C++ | `bash build.sh` (downloads and builds) |
| [imports](imports/) | Every import kind in one program | a C compiler |

## Setup

Install pyrock first (see the [main README](../README.md#installation)).

The `huggingface` and `langchain` examples import Python packages, which pyrock loads
through CPython. Their READMEs use a virtualenv at `examples/.venv`:

```sh
cd examples
python3 -m venv .venv                       # or: uv venv .venv
.venv/bin/pip install -r requirements.txt   # or: uv pip install --python .venv/bin/python -r requirements.txt
```

`raylib`, `raylib-breakout` and `opencv-yolo` load native libraries from their `lib/`
folders; each has a `build.sh` that downloads or builds them (nothing binary is committed).
`fetch-raylib.sh` downloads the official raylib 5.5 Linux x86-64 release. The examples are
written for Linux x86-64.

Third-party files kept in this folder: `raylib/raylib.h` (raylib 5.5, zlib licence) and
`opencv-yolo/include/onnxruntime_*.h` (ONNX Runtime 1.32 C API, MIT licence).

`scripts/readme_check.sh` in the repository root runs every example's README commands; in
CI it skips the examples marked `skip-ci`, which need a display, a camera, Ollama or the
virtualenv.
