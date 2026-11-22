# Type annotations: optional, partial, and used to unbox Python values.
from __future__ import annotations

import json
from typing import Optional


def area(w: float, h: float) -> float:
    return w * h


def add(a, b: int) -> int:
    return a + b


def parse_count(text: str) -> int:
    return json.loads(text)          # Python object unboxed to a native int here


def find(target: int, limit: int) -> Optional[int]:
    for i in range(limit):
        if i * i == target:
            return i
    return None


total: int = json.loads("40")        # native int from here on
print(area(2.5, 4.0), area(3, 4))    # ints stay ints, exactly as in CPython
print(add(total, 2), add(0.5, 1))
print(parse_count("[1, 2, 3]".replace("[1, 2, 3]", "7")) * 6)
print(find(49, 100), find(50, 100))
