# One program, four kinds of import. Resolution order for `import x`:
#   x.py (compiled natively) -> x.rs (Rust) -> x.c (C) -> CPython.
import json
import math
from collections import Counter

import geometry          # geometry.py: compiled natively
import vecmath           # vecmath.rs:  Rust, linked into the binary
import fastc             # fastc.c:     C, compiled with the system C compiler


def sum_squares(n):
    total = 0
    for i in range(n):
        total += i * i
    return total


area = geometry.circle_area(2.0)
print("circle area:", area)
print("rust dot:", vecmath.dot(1.5, 2.0, 3.0, 4.0))
print("rust greet:", vecmath.greet("pyrock"))
print("c gcd:", fastc.gcd(1071, 462))
print("c scale:", fastc.scale(2.5, 4))
print("c version:", fastc.version())

# CPython: stdlib modules (json is pure Python, math is a C extension).
print("math.sqrt:", math.sqrt(sum_squares(10)))
report = {"sum": sum_squares(1000), "area": area, "tags": ["native", "rust", "c"]}
print(json.dumps(report, sort_keys=True))

# Python objects keep Python semantics, including aliasing.
words = ["a", "b", "a", "c", "a"]
alias = words
alias.append("b")
counts = Counter(words)
print(len(words), counts.most_common(2))

# Values coming back from Python can be passed to Rust/C (checked conversion).
n = json.loads("12")
print("from json to C:", fastc.gcd(n, 18))
for w in sorted(set(words)):
    print(w, counts[w])
