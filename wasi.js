// Runs a WebAssembly program built by `pyrock build --target wasm32-wasip1`:
// the few WASI calls such a program makes (output, clocks, random numbers,
// exit), with no files, network or environment. Used by the playground's
// worker and, in scripts/build_site.sh, by Node to check each program's output
// against CPython before it is published.
//
//   runWasi(module, { stdout(bytes), stderr(bytes), args }) -> exit status
(function (global) {
  "use strict";
  var ESUCCESS = 0, EBADF = 8, ENOSYS = 52;

  function Exit(code) { this.code = code; }

  function runWasi(module, io) {
    var memory = null;
    var args = (io.args || ["main"]).map(function (a) { return new TextEncoder().encode(a + "\0"); });
    var view = function () { return new DataView(memory.buffer); };
    var bytes = function () { return new Uint8Array(memory.buffer); };
    var now = typeof performance !== "undefined" ? function () { return performance.now(); } : function () { return Date.now(); };
    var fill = typeof crypto !== "undefined" && crypto.getRandomValues
      ? function (b) { for (var i = 0; i < b.length; i += 65536) crypto.getRandomValues(b.subarray(i, i + 65536)); }
      : function (b) { for (var i = 0; i < b.length; i++) b[i] = (Math.random() * 256) | 0; };

    var wasi = {
      args_sizes_get: function (argc, size) {
        view().setUint32(argc, args.length, true);
        view().setUint32(size, args.reduce(function (n, a) { return n + a.length; }, 0), true);
        return ESUCCESS;
      },
      args_get: function (argv, buf) {
        args.forEach(function (a) {
          view().setUint32(argv, buf, true);
          bytes().set(a, buf);
          argv += 4;
          buf += a.length;
        });
        return ESUCCESS;
      },
      environ_sizes_get: function (count, size) {
        view().setUint32(count, 0, true);
        view().setUint32(size, 0, true);
        return ESUCCESS;
      },
      environ_get: function () { return ESUCCESS; },
      fd_write: function (fd, iovs, n, written) {
        if (fd !== 1 && fd !== 2) return EBADF;
        var total = 0, v = view();
        for (var i = 0; i < n; i++) {
          var ptr = v.getUint32(iovs + i * 8, true), len = v.getUint32(iovs + i * 8 + 4, true);
          var chunk = bytes().slice(ptr, ptr + len);
          (fd === 1 ? io.stdout : io.stderr)(chunk);
          total += len;
        }
        view().setUint32(written, total, true);
        return ESUCCESS;
      },
      fd_read: function (fd, iovs, n, read) {
        if (fd !== 0) return EBADF;
        view().setUint32(read, 0, true); // stdin is empty: end of file
        return ESUCCESS;
      },
      fd_fdstat_get: function (fd, stat) {
        if (fd > 2) return EBADF;
        var v = view();
        v.setUint8(stat, 2); // character device
        v.setUint16(stat + 2, 0, true);
        v.setBigUint64(stat + 8, 0n, true);
        v.setBigUint64(stat + 16, 0n, true);
        return ESUCCESS;
      },
      fd_close: function () { return EBADF; },
      fd_seek: function () { return EBADF; },
      fd_prestat_get: function () { return EBADF; },
      fd_prestat_dir_name: function () { return EBADF; },
      clock_res_get: function (id, res) {
        view().setBigUint64(res, 1000n, true);
        return ESUCCESS;
      },
      clock_time_get: function (id, precision, time) {
        var ns = id === 0 ? BigInt(Date.now()) * 1000000n : BigInt(Math.round(now() * 1e6));
        view().setBigUint64(time, ns, true);
        return ESUCCESS;
      },
      random_get: function (buf, len) {
        fill(new Uint8Array(memory.buffer, buf, len));
        return ESUCCESS;
      },
      sched_yield: function () { return ESUCCESS; },
      proc_exit: function (code) { throw new Exit(code); },
    };
    // Anything else (files, sockets, polling) isn't available.
    var imports = new Proxy(wasi, {
      get: function (t, name) { return name in t ? t[name] : function () { return ENOSYS; }; },
    });
    var instance = new WebAssembly.Instance(module, { wasi_snapshot_preview1: imports });
    memory = instance.exports.memory;
    try {
      instance.exports._start();
      return 0;
    } catch (e) {
      if (e instanceof Exit) return e.code;
      throw e; // a trap: the program aborted (a Rust panic)
    }
  }

  global.runWasi = runWasi;
  if (typeof module !== "undefined" && module.exports) module.exports = runWasi;
})(typeof self !== "undefined" ? self : globalThis);
