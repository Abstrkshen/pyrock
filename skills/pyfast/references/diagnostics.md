# pyfast diagnostics and how to fix them

Format: `file.py:LINE:COL  CODE  error|warning: message`, followed by indented
`reason:` / note lines. `pyfast check` exits non-zero if there is any error.

## Frontend (P0xx–P1xx): the code can't be lowered

| Code | Meaning | Fix |
|---|---|---|
| P001 | Python syntax error (from CPython's parser) | Fix the syntax. Check that the `--python` interpreter version supports it |
| P002 | Frontend failure (python3 missing, protocol mismatch) | Make sure `python3` (or `--python`) runs |
| P003 | An imported `.rs` file doesn't parse | Fix the Rust syntax |
| P100 | Unsupported statement or expression: `match`, `for`/`while ... else`, `del name`, `except*`, complex literals, `**` in a dict literal, relative import, `import *`, a native module imported inside a function, a class with a CPython base class / metaclass / several bases, unsupported decorators, `globals()`/`locals()`/`eval` without a namespace | Rewrite (see language.md), or move that code into an imported `.py` so hybrid mode imports it through CPython (P220) |
| P110 | Integer literal outside i64 | Use a float, or compute it in a CPython module |
| P140 | `mod.NAME` / `from mod import NAME` where the native module has no function, class or variable NAME | Define it in that module |
| P141 | A module-level function or class is defined twice, or a name rebinds a function or import | Use a different name |
| P142 | `global`/`nonlocal` of a function, class or import name | Use a separate variable |
| P143 | Undefined name | Define or import it |
| P144 | Function called at module level before its `def` | Move the `def` above the call |
| P145 | Circular import between native `.py` modules | Break the cycle |
| P150 | `mod.f` isn't a callable Rust/C function (missing, not `pub`/`static`, or its signature was skipped; the note says why) | Make it `pub`/non-static and use supported types (imports.md) |

## Boundaries (P2xx): errors only with `--mode native-only`

In hybrid mode these aren't printed by `check`. They're listed under "CPython boundaries" in
`explain`, and they work at runtime through CPython.

| Code | Boundary | Native alternative |
|---|---|---|
| P200 | CPython import | Put a `.py`/`.rs`/`.c` of that name next to the file, or remove the import |
| P210 | Attribute or method access on a Python object, calling a Python object, builtins with no native version (`getattr`, `open`, `type`, `vars`, ...) | Native functions, classes or Rust/C functions |
| P211 | A builtin applied to a Python object | Unbox first (an `int`/`str`/`bool` annotation) |
| P212 | A container that escapes to CPython (built as a Python object), subscript/slice of a Python object, item or attribute assignment on one | Keep the container native and pass a copy or its elements |
| P213 | Operator or comparison on a Python object | Keep operands native |
| P214 | `for x in <python iterable>` | Iterate a native list/range/generator |
| P215 | Value boxed (no single native type) | Make the variable or return single-typed. `T \| None`, `int \| float`, unions of native classes and string-keyed dicts stay native |
| P216 | `with` on a Python context manager, or `except` naming a Python exception class | Native exception classes; a native class with `__enter__`/`__exit__` |
| P217 | `await` of a Python awaitable, `async with`, `async for` | Inherent to asyncio: keep the CPU work between awaits native |
| P220 | Warning: local `.py` imported through CPython because it can't be compiled. The note shows the first blocker | Fix that blocker in the module |
| P231 | A Rust/C struct handle is passed to CPython (an error in both modes) | Pass its fields instead |
| P308 | Warning: an attribute that is never assigned anywhere is read (raises AttributeError at runtime) | Fix the attribute name |

## Types (P3xx)

| Code | Meaning | Fix |
|---|---|---|
| P301 | Operator on types where CPython raises `TypeError` (`1 + "a"`) | It's a real bug: fix the types |
| P302 | Value-dependent type (e.g. `int ** n` with non-literal `n` in native-only mode) | Use a float base or a literal exponent |
| P303 | Wrong argument count or type to a function, builtin, or Rust/C function | Fix the call |
| P304 | Local bound to two types in one specialization (native-only) | Use separate variables |
| P305 | Returns different types or sometimes None (native-only) | Return one type on every path |
| P306 | No return type could be inferred (e.g. infinite recursion with no base case) | Add a base-case return |
| P307 | Warning: native value contradicts an annotation. It keeps its runtime type, like CPython | Fix the annotation or the call |
| P310 | More than 16 specializations of one function. Hybrid mode then shares one all-`object` specialization (bridged) for the remaining calls; native-only reports it | Call it with fewer type combinations |
| P311 | Inference didn't converge | Report it as a pyfast bug, with a minimal program |

## Runtime errors

Uncaught exceptions print:

```
Traceback (most recent call last):
  File "main.py", in <module>  [native frames elided by pyfast]
ZeroDivisionError: division by zero
```

The process then exits with status 1. The last line matches CPython. `OverflowError` from int
arithmetic means a result passed i64. CPython would promote it to a bigint, which is a
documented divergence: use floats, or keep that computation in CPython.
