# Working on the pyrock repo

Repo: https://github.com/Abstrkshen/pyrock (a Cargo workspace). Rust 1.96, PyO3 0.30,
Python 3.12. In an agent shell, run `export PATH=$HOME/.cargo/bin:$PATH` first.

## Pipeline and crates

```
main.py → frontend (python3 -I dump_ast.py → JSON) → HIR (ir)
        → infer: generic schemes (explain) + specialization fixed point
        → analysis: tiers, effects, recursion, CPython boundaries
        → codegen-rust: main.rs (+ build.rs for C) → driver: Cargo.toml, cargo build
```

| Crate | Key files |
|---|---|
| `crates/frontend` | `dump_ast.py` (CPython AST → JSON protocol v1). `lib.rs`: `parse_file`, the Loader/Lowerer, import resolution (`resolve_local`, `resolve_import`), annotations, f-string/slice lowering. `externs.rs`: Rust (`syn`) and C signature extraction |
| `crates/ir` | `hir.rs` (Module, Unit, Function, StmtKind, ExprKind, ExternModule, ExtTy, PySlot), `types.rs` (Ty), `diag.rs` (Diagnostic), `lib.rs` (Span, Mode) |
| `crates/infer` | `ops.rs` (operator/builtin typing table), `spec.rs` (`specialize`, Program, Spec, mangling, P3xx), `definite.rs` (maybe-unbound), `generic.rs` (HM-style schemes) |
| `crates/analysis` | `plan`: call graph, recursion, effects, Native/Bridge tier, boundary list (P2xx) |
| `crates/codegen-rust` | `emit`: one fn per Spec, slots, unit inits, extern FFI, `rid()` keyword escaping |
| `crates/runtime` | `pyrock-runtime`, linked into generated programs: `int.rs`, `float.rs`, `string.rs`, `conv.rs`, `exc.rs`, `io.rs`, `ext.rs`, `py.rs` (PyO3 bridge, `python` feature), `stackful.rs` (corosensei coroutines), `gen.rs` (generators), `aio.rs` (async) |
| `crates/driver` | The `pyrock` CLI (`main.rs`): commands, Cargo.toml/build.rs generation, shared target dir, crate-name hashing, explain rendering |

Docs: `README.md`, `docs/imports.md`, `docs/how-it-works.md`, `docs/subset.md`, `Project.md` (spec).

## Adding a feature: the usual path

1. Lower it in `frontend/src/lib.rs` (new HIR node in `ir/src/hir.rs` if needed).
2. Type it in `infer/src/ops.rs` / `spec.rs`, and give it a generic constraint in `generic.rs`.
3. If it touches Python objects, add a boundary in `analysis`.
4. Emit it in `codegen-rust`, and put the semantics in a `runtime` function rather than inline.
5. Add a differential test, and update `docs/subset.md` and this skill.

## Tests

```sh
cargo test --workspace                                 # everything
cargo test -p pyrock --test suites                     # integration suites only
UPDATE_SNAPSHOTS=1 cargo test -p pyrock --test suites  # re-bless golden/diagnostic snapshots
bash samples/run_all.sh [--release] [name-filter]      # every sample vs CPython / expected files
```

* `tests/differential/*.py`: compiled output must equal `python3` on stdout, exit code and the
  last stderr line. Files starting with `_` are helper modules, not tests.
* `tests/golden/*.py`: snapshots `.rs.golden` and `.explain.golden`.
* `tests/diagnostics/*.py`: `.stderr` snapshot of `pyrock check`. A `# mode: native-only`
  first line selects that mode.
* `tests/interop/<name>/main.py`: Rust/C imports, compared with `expected.stdout` /
  `expected.stderr`.
* Tests use `PYROCK_TARGET_DIR=target/pyrock-tests/target`.

Review snapshot diffs before re-blessing. A changed golden is a behaviour change.

## Installing and maintainer shortcuts

`cargo install --path crates/driver` installs `pyrock` into `~/.cargo/bin`; run it again
after changing the compiler. `commands.perch` holds the maintainer's
`perch` shortcuts:

| Command | Action |
|---|---|
| `perch install` | `cargo build --release -p pyrock`, then install that binary |
| `perch build` | release build only |
| `perch tests` | `cargo test --workspace` |
| `perch samples` | `bash samples/run_all.sh` |
| `perch skill` | link this skill into `~/.claude/skills`, `~/.codex/skills` and `~/.cursor/skills` (`bash skills/install.sh --copy` / `--remove` for copies or uninstall) |

(perch reserves `test` and doesn't accept hyphens in command names.)

## Environment variables

| Variable | Effect |
|---|---|
| `PYROCK_TARGET_DIR` | Cargo target dir (default `$XDG_CACHE_HOME/pyrock/target` or `~/.cache/pyrock/target`) |
| `PYROCK_RUNTIME` | Path to a runtime crate (default: the copy embedded in the binary by `crates/driver/build.rs`, extracted to `<cache>/pyrock/runtime/<content hash>/`) |
| `UPDATE_SNAPSHOTS=1` | Re-bless test snapshots |

## Where the project stands

`docs/status.md` summarizes the current features, measured speedups and known differences;
`docs/roadmap.md` logs every step (what changed, how it was verified). Update both when a
change is user-visible.

`docs/coverage.md` says how much of Python compiles. Re-measure after a language change
with `python3 scripts/coverage.py` (the standard library by default, or any directory of
`.py` files) and update the numbers there, in `docs/status.md` and in the README.
