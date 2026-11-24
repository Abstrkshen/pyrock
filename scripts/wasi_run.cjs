// Run a WebAssembly program built by `pyrock build --target wasm32-wasip1`
// with the playground's own WASI shim (site/wasi.js):
//
//   node scripts/wasi_run.cjs program.wasm
//
// Prints its stdout and stderr, exits with its status; the run time goes to
// stderr's last line when PYROCK_WASI_TIME is set.
const fs = require("fs");
const path = require("path");
const runWasi = require(path.join(__dirname, "..", "site", "wasi.js"));

const mod = new WebAssembly.Module(fs.readFileSync(process.argv[2]));
const t0 = performance.now();
const code = runWasi(mod, {
  stdout: (b) => fs.writeSync(1, b),
  stderr: (b) => fs.writeSync(2, b),
});
if (process.env.PYROCK_WASI_TIME) fs.writeSync(2, `wasm: ${(performance.now() - t0).toFixed(1)} ms\n`);
process.exitCode = code;
