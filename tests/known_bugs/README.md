# Known compiler bugs

Programs that pyrock currently gets wrong. They are not run by the test suites; each one
should become a differential test once it is fixed. Found while writing the playground's
bigger programs (`examples/playground/`), which work around them. Tracked in
`docs/roadmap.md` (phase 8, known bugs).

| File | Problem | `pyrock check --mode native-only` |
|---|---|---|
| `isinstance_union_range_len.py` | Inside `if isinstance(v, A):`, `v` still has the whole union's type: reading `v.xs` warns P308 for the other class, and `range(len(v.xs))` is rejected | P308 warnings, P301 error (`'Never' object cannot be interpreted as an integer`) |
| `isinstance_union_invalid_rust.py` | The same pattern in a larger program passes `check`, but the generated Rust doesn't compile (a block expression in a `for` header) | passes; `pyrock build` fails in rustc |
| `zip_collapses_inference.py` | `for name, value in zip(f.params, args)` makes inference give up on the whole program (every value becomes `object`); the same loop with indexing compiles | P215, P213, P210, P212, P214 errors |
| `method_overrides_collapse.py` | Overriding methods across a class hierarchy (the base version raises) collapses inference to `object`, starting with P302 on `[args[0]] + args[1].as_list()` | P302 and follow-up errors |

A performance note from the same work: a generator expression costs about 6 µs per
creation natively, so `" ".join(f"..." for ...)` in a hot loop can be slower than
CPython; a list comprehension is not.
