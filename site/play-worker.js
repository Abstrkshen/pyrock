// The playground's worker: Pyodide (CPython in WebAssembly) parses and runs
// Python; pyrock.wasm (the pyrock compiler, built from crates/playground)
// does everything between parsing and Rust code. It runs off the page, so a
// program that never ends can be stopped by terminating the worker.

const PYODIDE = "https://cdn.jsdelivr.net/pyodide/v0.27.7/full/";
importScripts(PYODIDE + "pyodide.js");

let py = null;      // Pyodide
let rock = null;    // pyrock.wasm exports
let ready = null;

function readOut() {
  return new TextDecoder().decode(new Uint8Array(rock.memory.buffer, rock.out_ptr(), rock.out_len()));
}

async function loadCompiler() {
  const res = await fetch("pyrock.wasm");
  if (!res.ok) throw new Error("pyrock.wasm: HTTP " + res.status);
  const { instance } = await WebAssembly.instantiate(await res.arrayBuffer(), {});
  return instance.exports;
}

async function init() {
  const [p, r] = await Promise.all([loadPyodide({ indexURL: PYODIDE }), loadCompiler()]);
  py = p;
  rock = r;
  rock.dump_ast_source();
  py.globals.set("_pyrock_dump_src", readOut());
  rock.version();
  const version = readOut();
  py.setStdin({ stdin: () => null }); // input() sees end of file
  await py.runPythonAsync(`
import sys, io, contextlib, traceback, time

def _pyrock_parse(src):
    with open("/tmp/main.py", "w") as f:
        f.write(src)
    sys.argv = ["dump_ast.py", "/tmp/main.py"]
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        exec(compile(_pyrock_dump_src, "dump_ast.py", "exec"), {"__name__": "__main__"})
    return buf.getvalue()

def _pyrock_run(src):
    """Run like \`python3 main.py\`: the exit status, and a traceback on error."""
    with open("main.py", "w") as f:  # so tracebacks can show the source lines
        f.write(src)
    import linecache
    linecache.checkcache("main.py")
    code = compile(src, "main.py", "exec")
    status = 0
    try:
        exec(code, {"__name__": "__main__", "__file__": "main.py"})
    except SystemExit as e:
        status = e.code if isinstance(e.code, int) else (0 if e.code is None else 1)
        if e.code is not None and not isinstance(e.code, int):
            print(e.code, file=sys.stderr)
    except BaseException as e:
        tb = e.__traceback__.tb_next  # drop this harness's frame
        sys.stderr.write("".join(traceback.format_exception(type(e), e, tb)))
        status = 1
    sys.stdout.flush()
    sys.stderr.flush()
    return status
`);
  return { version, python: py.runPython("sys.version.split()[0]") };
}

function compileSource(src, nativeOnly) {
  const ast = new TextEncoder().encode(py.globals.get("_pyrock_parse")(src));
  const p = rock.alloc(ast.length);
  new Uint8Array(rock.memory.buffer, p, ast.length).set(ast);
  try {
    rock.compile_ast(p, ast.length, nativeOnly ? 1 : 0);
  } finally {
    rock.dealloc(p, ast.length);
  }
  return JSON.parse(readOut());
}

async function runSource(src, id) {
  // Output goes to the page in batches: Python runs synchronously here, so
  // nothing can flush on a timer; a batch is sent when it reaches 64 KB, when
  // output switches between stdout and stderr, and when the program ends.
  let pendingStream = null, pendingText = "";
  const flush = () => {
    if (pendingText) postMessage({ id, type: "stream", stream: pendingStream, text: pendingText });
    pendingText = "";
  };
  const writer = (stream) => {
    const dec = new TextDecoder();
    return {
      write(buf) {
        if (pendingStream !== stream) { flush(); pendingStream = stream; }
        pendingText += dec.decode(buf, { stream: true });
        if (pendingText.length > 65536) flush();
        return buf.length;
      },
    };
  };
  py.setStdout(writer("stdout"));
  py.setStderr(writer("stderr"));
  await py.loadPackagesFromImports(src, { messageCallback: () => {}, errorCallback: () => {} });
  const t0 = performance.now();
  const status = py.globals.get("_pyrock_run")(src);
  flush();
  return { status, ms: performance.now() - t0 };
}

onmessage = async (e) => {
  const { id, cmd, src, nativeOnly } = e.data;
  try {
    if (!ready) ready = init();
    const info = await ready;
    if (cmd === "init") {
      postMessage({ id, type: "done", result: info });
    } else if (cmd === "compile") {
      postMessage({ id, type: "done", result: compileSource(src, nativeOnly) });
    } else if (cmd === "run") {
      postMessage({ id, type: "done", result: await runSource(src, id) });
    }
  } catch (err) {
    postMessage({ id, type: "error", message: String(err && err.message || err) });
  }
};
