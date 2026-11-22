# pyfast frontend protocol v1.
# Parses a Python file with CPython's own `ast` module and writes a JSON tree
# to stdout. Every node becomes {"_t": <class name>, <fields>..., "_loc": [...]}.
# Constants are tagged so that big ints and float bit patterns survive JSON.
import ast
import builtins
import json
import sys

PROTOCOL = 1


def const(v):
    if v is None:
        return {"kind": "none"}
    if isinstance(v, bool):
        return {"kind": "bool", "value": v}
    if isinstance(v, int):
        return {"kind": "int", "value": str(v)}
    if isinstance(v, float):
        return {"kind": "float", "value": repr(v)}
    if isinstance(v, str):
        return {"kind": "str", "value": v}
    if isinstance(v, bytes):
        return {"kind": "bytes", "value": list(v)}
    return {"kind": "other", "type": type(v).__name__}


def conv(node):
    if isinstance(node, ast.AST):
        out = {"_t": type(node).__name__}
        # `async def` lowers like `def`, flagged (the HIR marks the function).
        # `async def` / `async with` / `async for` lower like their plain
        # forms, flagged.
        if isinstance(node, (ast.AsyncFunctionDef, ast.AsyncWith, ast.AsyncFor)):
            out["_t"] = type(node).__name__[5:]
            out["is_async"] = True
        for name, value in ast.iter_fields(node):
            if isinstance(node, ast.Constant) and name == "value":
                out["value"] = const(value)
            elif name in ("kind", "type_comment"):
                continue
            else:
                out[name] = conv(value)
        if hasattr(node, "lineno"):
            out["_loc"] = [node.lineno, node.col_offset + 1,
                           node.end_lineno or node.lineno, (node.end_col_offset or node.col_offset) + 1]
        return out
    if isinstance(node, list):
        return [conv(n) for n in node]
    return node


def main():
    path = sys.argv[1]
    with open(path, "rb") as f:
        src = f.read()
    try:
        tree = ast.parse(src, filename=path)
    except SyntaxError as e:
        json.dump({"protocol": PROTOCOL, "error": {"msg": e.msg, "line": e.lineno or 0,
                                                  "col": e.offset or 0}}, sys.stdout)
        return
    json.dump({"protocol": PROTOCOL, "python": sys.version.split()[0], "module": conv(tree),
               "builtins": [n for n in dir(builtins) if not n.startswith("_")]},
              sys.stdout)


main()
