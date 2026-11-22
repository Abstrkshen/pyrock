# Container annotations on variables give empty containers their element
# types (ollama-agents FINDINGS F-04/F-05).
from typing import Optional


class Step:
    def __init__(self, tool):
        self.tool = tool


def render(steps):
    parts: list[str] = []
    for s in steps:
        parts.append('{"tool":"' + s.tool + '"}')
    return "[" + ",".join(parts) + "]"


def index(words):
    counts: dict[str, int] = {}
    seen: set[str] = set()
    for w in words:
        counts[w] = counts.get(w, 0) + 1
        seen.add(w)
    best: Optional[str] = None
    for w in sorted(seen):
        if best is None or counts[w] > counts[best]:
            best = w
    return counts, best


REGISTRY: dict[str, Step] = {}
EMPTY: list[int] = []

print(render([]), render([Step("a"), Step("b")]))
print(index(["x", "y", "x"]), index([]))
REGISTRY["calc"] = Step("calculate")
print(sorted(REGISTRY), REGISTRY["calc"].tool, len(EMPTY), sum(EMPTY))
