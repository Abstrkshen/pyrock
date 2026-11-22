# `T | None` values narrowed by `is None` checks, truthiness, early exits,
# `and`/`or` and conditional expressions (ollama-agents FINDINGS F-01).
def lookup(d, k):
    if k in d:
        return d[k]
    return None


def run(name: str, args: str) -> str:
    return name + "(" + args + ")"


def go(d):
    out = []
    for k in ["a", "b", "c"]:
        name = lookup(d, k)
        args = lookup(d, "args")
        if name is None:
            continue
        if args is None:
            args = ""
        out.append(run(name, args))
    return out


def joined(d):
    hist = []
    m = lookup(d, "message")
    if m is None:
        hist.append("none")
    else:
        hist.append(m.upper())
    n = lookup(d, "a")
    if n is not None and n.startswith("x"):
        hist.append(n)
    t = lookup(d, "b")
    if not t:
        return ",".join(hist)
    hist.append(t + "!")
    return ",".join(hist)


def first_word(s):
    w = lookup({"k": s}, "k") if s else None
    return w.split()[0] if w is not None else "-"


def loop_reassign(items):
    cur = None
    total = 0
    for x in items:
        if cur is not None:
            total += len(cur)
        cur = x if x != "" else None
    return total


print(go({"a": "x", "c": "z", "args": "1"}))
print(go({"b": "y"}))
print(joined({"message": "hi", "a": "xy", "b": "q"}), joined({}))
print(first_word("hello world"), first_word(""))
print(loop_reassign(["ab", "", "cde", "f"]))
