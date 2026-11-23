# pyrock findings while building ollama-agents
Compiler: pyrock at HEAD (target/release/pyrock). Repros are in `repro/` (run from that directory, e.g.
`pyrock check r8.py --mode native-only`). Nothing in pyrock was modified.

## F-01  Option narrowing is not flow-sensitive: `str | None` leaks (P301, P307, boxed results)
`net.json_get` returns `Option<String>` (`str | None`). Testing `is None` / `continue` / `else` does not narrow later uses.
`repro/r21.py`:
```python
def run(name: str, args: str) -> str: ...
name = net.json_get(doc, "/n"); args = net.json_get(doc, "/a")
if name is None: continue          # inside a loop
if args is None: args = "{}"
run(name, args)                    # warning P307 at both arguments
m = net.json_get("{}", "/message")
if m is None: print("none")
else: hist.append(m)               # hist becomes list[str | None]
",".join(hist)                     # error P301
```
```
r21.py:22:7  P301  error: sequence item 0: expected str instance, str | None found
r21.py:14:19  P307  warning: str | None value where the annotation says str
```
In the real program this cascaded (history list, `has_tool(t)`, dict keys `str | None for a dict[str, str]`, P301 on
`tools.py` dict lookups). Workaround: Rust helper `json_str(doc, ptr) -> String` returning "" when missing (Python
tests `== ""`), no Option in Python at all.

## F-02  Empty module-level dict filled inside functions makes the value type unknown (P215)
`repro/r1.py` (also r5 with an annotation, r6 wrapped in a class, r7 filled in a function: all fail):
```python
AGENTS = {}
AGENTS["a"] = Agent()
def get(name): return AGENTS[name]
a = get("a"); a.chat(2)
for s in a.steps: print(s.tool)
```
```
r1.py:1:1  P215  error: variable `s` holds object and Never in <module>
r1.py:24:11  P210  error: attribute `.tool` in <module>
```
`AGENTS: dict[str, Agent] = {}` is ignored. `repro/r4.py` (dict literal seeded with an instance) passes.
Workaround: seed the registry with one real built-in agent (`AGENTS = {"assistant": Agent(...)}`).

## F-03  List attribute appended in one method, read in another: element type unresolved (P215 / P213)
`repro/r14.py` (r15, r16, r17, r19 are variants; none helps):
```python
class Agent:
    def __init__(self): self.steps = []
    def chat(self, n):
        for i in range(n): self.steps.append(Step("t" + str(i)))
    def steps_json(self):
        out = ""
        for s in self.steps:
            out += s.to_json()        # to_json returns str
        return out
```
```
r14.py:18:5  P215  error: variable `out` holds str and object in Agent.steps_json(Agent)
```
Same program passes when the list is seeded (`self.steps = [Step("")]`, r18) or when the loop is in a free function taking
the list as a parameter (`render(self.steps)`, r20). Used r20 in `agents.py::render_steps`. Related: `explain` printed
generic return types `agents.Agent.steps_json(A) -> object` and listed bridged specializations until fixed.

## F-04  `pyrock check` (even `--mode native-only`) passes but the generated Rust does not compile (E0308)
`repro/r8.py` (r9/r10/r12 same; r11 string `+=` works):
```python
class Step:
    def __init__(self, tool): self.tool = tool
    def to_json(self): return '{"tool":"' + self.tool + '"}'
class Agent:
    def __init__(self): self.steps = []
    def chat(self, n):
        for i in range(n): self.steps.append(Step("t" + str(i)))
    def steps_json(self):
        parts = []
        for s in self.steps: parts.append(s.to_json())
        return "[" + ",".join(parts) + "]"
```
```
error[E0308]: mismatched types   --> src/main.rs:839:104
  expected `&[Rc<str>]`, found `&Vec<Obj>`     (rt::string::join(&rt::lit!(","), &parts...))
pyrock: cargo build of the generated project failed (... otherwise this is a pyrock bug)
```
Generated: `let mut parts: rt::List<rt::py::Obj>;` and `parts.append(rt::py::str_(&agents_Step_to_json(..)?))`. Same root cause as
F-03 (element type unresolved) but here check reported `ok (native-only) ... 0 bridged` while codegen boxed the list.
Local annotation `parts: list[str] = []` is ignored. Workaround: accumulate with `out += ...`, then F-03 fix.

## F-05  Annotations on locals are ignored
`parts: list[str] = []` had no effect on the generated type (`rt::List<rt::py::Obj>` stayed), see F-04. Only parameter/return
annotations are used (as documented), but the docs do not say local annotations are no-ops.

## Not problems (worked first time, worth noting for the video)
* Rust `Result<(i64,String,String,String),String>` returned to Python tuple-unpacking: `conn, method, path, body = net.accept()`.
* Rust `Err` raised `RuntimeError`, caught natively with `except RuntimeError as e`.
* Dict of function values (`TOOLS = {"calculate": calculate, ...}`; `fn = TOOLS[name]; fn(args)`) compiled natively.
* Classes, recursive-descent parser class, module-level `NOTES` dict mutated from tool functions, `str.strip(chars)`, `str.lower().split()`, slices,
  `startswith/endswith`, `float(...)`, `del d[k]` all native.
* Cold vs warm: first release build 7.6 s; clean rebuild (`rm -rf .pyrock`, shared cargo cache warm) 4.2 s; binary 955 KB, no libpython.
