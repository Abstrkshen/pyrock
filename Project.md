> **Implementation status (2026-11-23):** this is the original design draft and is kept as
> written. What was built, and where it differs from this draft, is in
> [docs/status.md](docs/status.md) ("Compared with Project.md"); open items are phase 8 of
> [docs/roadmap.md](docs/roadmap.md).

Draft v0.1 · 21 November 2026

Build a Python-to-Rust ahead-of-time compiler that infers generic functions from ordinary Python source, specializes them into native machine code, and embeds CPython for external libraries and genuinely dynamic behavior. The project is intended as an independent, open-source engineering experiment for abstrkshen.dev.

1. Goals and non-goals

Accept ordinary .py files; no mandatory annotations or decorators.

Generate readable Rust source and executable binaries.

Eliminate Python object dispatch wherever static analysis proves native operations valid.

Preserve access to installed Python packages using embedded CPython through PyO3.

Explain every compilation decision and measure its effect.

The initial release is not a complete Python implementation, a sandbox, a replacement for pip, or a promise that all programs will become faster. C emission is a later backend, not an MVP requirement.

2. Architecture

flowchart TD
 A[Python source] --> B[Parser and module resolver]
 B --> C[HIR and scope analysis]
 C --> D[Type and effect inference]
 D --> E[Typed IR and specialization]
 E --> F[Native eligibility and escape analysis]
 F --> G[Native Rust emitter]
 F --> H[CPython bridge emitter]
 G --> I[Generated Cargo project]
 H --> I
 I --> J[Native executable]
 J <-->|Python objects| K[Embedded CPython]

Subsystem

Responsibility

Frontend

Parse syntax, preserve locations, resolve lexical bindings and imports.

HIR

Desugar Python control flow without changing evaluation order.

Inference

Infer types, generic constraints, effects and return types.

Specializer

Instantiate functions for reachable concrete type combinations.

Eligibility planner

Determine native, bridged or interpreted regions.

Rust backend

Emit source, ownership operations, error propagation and adapters.

Runtime

Manage CPython lifecycle, object handles, conversions and exceptions.

Driver

Build, cache, run, explain and benchmark.

3. End-to-end example

import json

def square(x):
    return x * x

def sum_squares(n):
    total = 0
    for i in range(n):
        total += square(i)
    return total

print(square(2.5))
print(sum_squares(100000))
print(json.dumps({"result": sum_squares(100)}))

The compiler recognizes two instantiations of square, integer arithmetic in the loop, and a Python boundary at json.dumps. The dictionary must be represented as a Python object at the boundary. Native integer arithmetic uses a representation that preserves Python integer semantics or a separately documented fixed-width subset.

Illustrative generated Rust with runtime helper APIs (not a complete implementation):

use pyo3::prelude::*;
use pyo3::types::PyDict;

fn square<T: Copy + std::ops::Mul<Output = T>>(x: T) -> T {
    x * x
}

// For this bounded example the compiler proves all values fit i64.
fn sum_squares(n: i64) -> i64 {
    let mut total = 0_i64;
    for i in 0..n { total += square(i); }
    total
}

fn main() -> PyResult<()> {
    Python::initialize();
    println!("{}", square(2.5_f64));
    println!("{}", sum_squares(100000));
    Python::attach(|py| {
        let json = py.import("json")?;
        let data = PyDict::new(py);
        data.set_item("result", sum_squares(100))?;
        let output: String = json.getattr("dumps")?.call1((data,))?.extract()?;
        println!("{}", output);
        Ok(())
    })
}

Production emission must preserve Python formatting where observable (e.g. float printing), prove integer bounds per specialization or use checked/promoting arithmetic, and preserve import order. The example is a sketch of the division of labor, not a claim of general semantic equivalence.

4. Compilation contract

Mode

Behavior

hybrid (default)

Compile eligible regions natively; lower other operations to CPython or execute intact Python regions.

native-only

Fail compilation when a required operation cannot be implemented without CPython.

strict

Reject unsupported source or dynamic fallback; combine with either target mode.

Fallback is a compile-time decision, not catch-all exception handling. Never run a native function, catch its failure, and rerun the original function: it could duplicate side effects. A guarded native fast path may fall back only before observable effects or inside a specifically designed resumable operation.

5. Generic inference and specialization

An unannotated parameter initially receives a type variable; it is not automatically a dynamic Python object. Operators introduce relational constraints.

def add(a, b):
    return a + b

def twice(x):
    return add(x, x)

print(twice(12))
print(twice(1.5))
print(add("a", "b"))

Inferred signature: add(A, B) -> R where Add(A, B, R). twice(T) -> R where Add(T, T, R). The compiler resolves each concrete call against a Python-semantic operator table. It can emit a Rust generic when constraints have equivalent Rust traits, or specialized functions when they do not (e.g. mixed numeric promotion or string concatenation).

Algorithm:

Build lexical symbol tables and a conservative call graph.

Allocate type variables for parameters and local values; generate constraints from expressions and assignments.

Propagate types through control-flow joins and function returns until a fixed point.

Starting at module entry, enqueue reachable calls keyed by function ID, argument types and semantic mode.

Solve constraints for each specialization; enqueue callees. Install placeholders for recursion; use strongly connected components to resolve recursive return types.

Cap specialization count and inference iterations. If a type remains unknown, box it or reject it in native-only mode.

Emit generic Rust only when its trait laws and operations match the source semantics; otherwise emit monomorphized Rust functions.

A future C backend emits one concrete function per specialization, using the same IR.

Example specialization report:

add(int,int)       -> int      native
add(float,float)   -> float    native
add(str,str)       -> str      native concat helper
twice(int)         -> int      native
json.dumps(dict)   -> PyObject bridge

6. Type system and semantics

IR type

Representation / rule

Bool

Rust bool; account for Python bool-as-int behavior.

Int

Arbitrary precision, or proven i64 fast path with promotion on overflow.

Float

f64 with Python-compatible error and comparison behavior.

String

Native representation only with equivalent Unicode semantics; otherwise PyObject.

Tuple

Fixed typed product; preserve identity when observable.

List[T]

Native Vec only if aliasing, mutation and escape analysis prove it safe.

Dict[K,V]

Ordered map with compatible hashing/equality, or PyObject.

Optional[T]

Option<T> only where None is the sole alternate value.

Union

Closed enum when all alternatives and operations are known.

PyObject

Owned Py<PyAny> or scoped Bound handle.

Sequential rebinding can infer different SSA types. Branch joins require a compatible common type, a tagged union or boxing. Generics alone cannot represent a value whose type changes unpredictably at runtime.

def choose(flag):
    if flag:
        return 42
    return "hello"

This produces a closed IntOrString union when its uses support that representation, otherwise a Python object. It is not a generic return type chosen by the caller.

7. Memory and identity

Python assignment aliases mutable objects. Rust assignment often moves values; naïve cloning changes behavior.

a = [1, 2]
b = a
b.append(3)
print(a)  # [1, 2, 3]

MVP policy: keep mutable collections boxed unless analysis proves a unique, nonescaping native allocation. Later, introduce borrowed views or shared native handles with identity-preserving semantics. Track escape through return values, object fields, closures, unknown functions and Python imports. Once a value escapes to Python, keep its canonical object representation unless a safe explicit conversion is possible.

Do not copy a Python list to a Vec and assume later mutations are reflected. Copying, borrowing and shared identity are distinct boundary operations. Untrusted Python extension code is outside Rust's memory-safety guarantee.

8. CPython interoperability

Use PyO3 rather than manually managing raw Python.h pointers. It calls the CPython C API and handles Python object lifetimes and error results. Raw FFI may be isolated in an audited runtime module if necessary.

Initialize one interpreter per process for the MVP; configure module paths and environment explicitly.

Keep owned Python handles across calls; use scoped attached handles during Python operations.

Import external packages at runtime, with normal Python import semantics. Do not execute package code at build time just to infer types.

Translate unknown calls via attribute lookup and Python callable invocation, preserving positional and keyword arguments.

Represent unknown results as Python objects until a proven conversion or explicit annotation/check establishes a native type.

Attach to Python when required by PyO3; avoid holding interpreter synchronization during long pure Rust computations. Do not assume every extension supports free-threaded execution.

Preserve Python exceptions as PyErr, mapped to a unified internal runtime error when crossing native code.

If interpreted Python calls compiled functions, register wrapper functions or a generated module. These wrappers convert arguments, apply type guards and dispatch to compatible specializations. Unsupported signatures use the original implementation if available.

Interop is bidirectional but not a security boundary. Imported Python packages can perform arbitrary permitted process operations.

9. Intermediate representation and repository

Function { id, params, return_type, blocks, effects }
Block { id, instructions, terminator }
Value { id, type, span, representation }
Instruction = Const | Add | CallNative | CallPython
            | Box | UnboxChecked | LoadAttr | Branch
            | Phi | Return | Raise
Effects = Pure | MayRaise | ReadsGlobal | Mutates | CallsPython

Track source spans, object identity requirements, evaluation order and exceptional edges. Verify the IR before emission: all values are defined, call signatures agree, native instructions have resolved types, and Python boundaries are explicit.

pyrust/                         # working name
  crates/
    driver/                     # CLI and build orchestration
    frontend/                   # parser adapter, scopes, HIR
    ir/                         # typed IR and verifier
    infer/                      # constraints, specialization
    analysis/                   # effects, escape and eligibility
    codegen-rust/               # Rust source emission
    runtime/                    # native semantics and PyO3 bridge
  tests/
    differential/
    diagnostics/
    golden/
    interop/
  examples/
  benchmarks/
  docs/

Bootstrap using CPython's ast parser via a small versioned frontend protocol, or use a Rust parser with compatibility tests. Keep parsing independent of type analysis. An eventual Capy frontend is possible but not required for this independent public project.

10. CLI and generated artifacts

pyrust check main.py
pyrust explain main.py --format text
pyrust emit main.py --target rust --out generated/
pyrust build main.py --mode hybrid --release
pyrust run main.py --mode hybrid -- arg1 arg2
pyrust build main.py --mode native-only
pyrust bench main.py --baseline python

Output: generated Cargo.toml, Rust sources, source map, compilation report, executable and dependency manifest. Use Cargo/rustc rather than implementing machine-code emission. Cache by source/module content, compiler version, semantic mode, target, ABI and dependencies. Pin the supported CPython version initially; use the matching interpreter, shared library and Python environment at runtime. Embedding does not automatically make third-party packages portable or create a single deployment file.

11. MVP scope and diagnostic policy

Support functions, local variables, fixed positional arguments, literals, arithmetic, comparisons, if/else, while, range loops, return, basic print, and runtime imports/calls. Support recursion after basic inference works. Native collections, classes, comprehensions, closures and keyword calls can follow.

Reject or interpret unsupported syntax as an intact region. Never silently lower eval, exec, monkey patching, dynamic imports or global rebinding into a native assumption. Preserve source locations and explain why a region is boxed.

main.py:14:8  P210  dynamic receiver type
  selected CPython attribute lookup
  reason: result of unannotated external function
  native-only: compilation error

Provide a machine-readable report with specialization IDs, native/bridge counts, source ranges, inferred types, reasons for fallback and runtime requirements.

12. Verification and benchmarks

Test family

Required check

Differential

Same input, CPython versus generated binary: output, exit status, exceptions and observable mutations.

Numeric

Overflow, negative floor division/modulo, division by zero, bool/int interactions, NaN and signed zero.

Generic

Multiple concrete calls, mixed operands, recursion, invalid operators and specialization limits.

Interop

Import json/math and external packages; exceptions, returned handles and callbacks.

Identity

Aliased lists, escaped objects and mutations across boundaries.

Golden

Stable readable Rust, typed IR and diagnostics snapshots.

Build

Clean and cached builds on Linux and macOS.

Run benchmarks separately for compilation time, startup latency, steady-state runtime, peak RSS, native code size and bridge overhead. Compare against equivalent CPython code with identical algorithms, inputs and output checks. Include workloads that do not benefit and workloads dominated by native Python packages. Report medians, dispersion, hardware, interpreter/compiler versions and commands. Do not advertise speedup until measured.

13. Delivery roadmap

Milestone

Demonstrable outcome

Acceptance gate

M0

Python AST to simple Rust source

Arithmetic and control-flow golden tests pass.

M1

Inferred numeric generics

Same function called with int and float; differential tests pass.

M2

Embedded CPython

Import json, call functions and propagate exceptions.

M3

Hybrid planning

One program mixes native loop and Python package; explain report identifies both.

M4

Semantic hardening

Integer semantics, operator edge cases, aliasing fallback and native-only refusal.

M5

Release and benchmarks

Reproducible build, documentation, tests and published measured results.

M6

Broader subset

Collections, classes, closures, callbacks; optional C backend.

M0–M3 form the first public prototype. Do not attempt full Python compatibility before this vertical slice works.

14. YouTube production plan

First video: I Turned Python Functions Into Rust Generics.

Hook: run one ordinary, unannotated Python function with several types; show the generated Rust specializations.

Problem: visualize runtime object dispatch versus compile-time constraints.

Build: animate the AST, inferred types and native-versus-Python decisions.

Demo: type commands on screen, compile a native numeric loop, then call an existing Python package in the same program.

Result: compare verified output and real benchmark results; show a case that must remain dynamic.

Close: link the standalone repository and invite viewers to submit Python programs that challenge the compiler.

Subsequent episodes can each solve a genuine limitation: Python integer overflow, shared list mutation, importing NumPy, callbacks and eliminating interpreter dependencies.

Keep this repository separate from Capyler and other proprietary projects. The contribution is an educational implementation with inspectable generated code and explicit compilation decisions, not a claim that hybrid Python compilation is unprecedented.

15. Technical references

CPython embedding guide

CPython interpreter lifecycle

PyO3 guide: calling Python from Rust

PyO3 guide: building and distribution

Shed Skin: restricted Python to C++ compiler

These are technical reference points, not evidence that the proposed compiler is already implemented.