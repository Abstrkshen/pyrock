from __future__ import annotations

import json
from typing import Optional, List


def add(a: int, b: int) -> int:
    return a + b


def add_partial(a, b: int) -> int:
    return a + b


def scale(x: float, k: float) -> float:
    return x * k


def parse_int(s: str) -> int:
    return json.loads(s)


def ratio(n: "int", d: "float") -> "float":
    return n / d


def describe(name: str) -> str:
    return name


def maybe(flag: bool) -> Optional[int]:
    if flag:
        return 1
    return None


def total(xs: List[int]) -> int:
    t = 0
    for x in xs:
        t += x
    return t


def log(msg: str) -> None:
    print("log:", msg)


count: int = json.loads("41")
label: str = json.loads('"widgets"')
weight: float = json.loads("2")
print(add(1, 2), add_partial(1.5, 2), add_partial(count, 1))
print(parse_int("12") + 1, scale(2, 3), scale(1.5, 2.0), ratio(1, 4))
print(label + "!", weight, weight / 4, count * 2)
print(maybe(True), maybe(False), total([1, 2, 3]))
log("done")
print(add("not", "checked"))
