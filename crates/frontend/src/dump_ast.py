# pyrock frontend protocol v1.
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


ENUM_KINDS = ("Enum", "IntEnum", "StrEnum")


def enum_aliases(tree):
    """Names that refer to enum.Enum / IntEnum / StrEnum / auto in this module."""
    names, modules = {}, set()
    for st in tree.body:
        if isinstance(st, ast.ImportFrom) and st.module == "enum" and st.level == 0:
            for a in st.names:
                if a.name in ENUM_KINDS + ("auto",):
                    names[a.asname or a.name] = a.name
        elif isinstance(st, ast.Import):
            for a in st.names:
                if a.name == "enum":
                    modules.add(a.asname or "enum")
    return names, modules


def enum_ref(node, names, modules):
    if isinstance(node, ast.Name):
        return names.get(node.id)
    if isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name) and node.value.id in modules:
        return node.attr if node.attr in ENUM_KINDS + ("auto",) else None
    return None


ENUM_TEMPLATE = """
class {C}:
    __pyrock_enum__ = "{K}"
    def __init__(self, _name_, _value_):
        self._name_ = _name_
        self._value_ = _value_
    @property
    def name(self):
        return self._name_
    @property
    def value(self):
        return self._value_
    def __repr__(self):
        return "<{C}." + self._name_ + ": " + repr(self._value_) + ">"
    def __str__(self):
        return {STR}
    @classmethod
    def _pyrock_lookup_(cls, value):
        for m in {C}._pyrock_members_:
            if m._value_ == value:
                return m
        raise ValueError(repr(value) + " is not a valid {C}")
    @classmethod
    def _pyrock_by_name_(cls, name):
        for m in {C}._pyrock_members_:
            if m._name_ == name:
                return m
        raise KeyError(name)
"""

# IntEnum / StrEnum members are also ints / strs: they compare (and IntEnum
# members compute) with plain values.
MIXIN_TEMPLATE = """
class _M:
    def __eq__(self, other):
        if isinstance(other, {C}):
            return self._value_ == other._value_
        return self._value_ == other
    def __lt__(self, other):
        if isinstance(other, {C}):
            return self._value_ < other._value_
        return self._value_ < other
    def __le__(self, other):
        if isinstance(other, {C}):
            return self._value_ <= other._value_
        return self._value_ <= other
    def __gt__(self, other):
        if isinstance(other, {C}):
            return self._value_ > other._value_
        return self._value_ > other
    def __ge__(self, other):
        if isinstance(other, {C}):
            return self._value_ >= other._value_
        return self._value_ >= other
"""

INT_TEMPLATE = """
class _M:
    def __int__(self):
        return self._value_
    def __index__(self):
        return self._value_
    def __add__(self, other):
        if isinstance(other, {C}):
            return self._value_ + other._value_
        return self._value_ + other
    def __radd__(self, other):
        return other + self._value_
    def __sub__(self, other):
        if isinstance(other, {C}):
            return self._value_ - other._value_
        return self._value_ - other
    def __rsub__(self, other):
        return other - self._value_
    def __mul__(self, other):
        if isinstance(other, {C}):
            return self._value_ * other._value_
        return self._value_ * other
"""


def relocate(nodes, at):
    for n in nodes:
        for x in ast.walk(n):
            if isinstance(x, (ast.stmt, ast.expr, ast.excepthandler, ast.arg, ast.keyword)):
                x.lineno, x.col_offset = at.lineno, at.col_offset
                x.end_lineno, x.end_col_offset = at.lineno, at.col_offset
    return nodes


def rewrite_enum(cls, kind, names, modules):
    """`class C(Enum)` as a plain class: members are instances created in the
    class body, plus the methods Enum provides. pyrock's frontend maps
    `C(v)`, `C[name]`, `for m in C`, `len(C)` and `m in C` to the helpers."""
    c = cls.name
    str_expr = "self._value_" if kind == "StrEnum" else ("str(self._value_)" if kind == "IntEnum" else '"%s." + self._name_' % c)
    new = ast.parse(ENUM_TEMPLATE.format(C=c, K=kind, STR=str_expr)).body[0]
    extra = []
    if kind in ("IntEnum", "StrEnum"):
        extra += ast.parse(MIXIN_TEMPLATE.format(C=c)).body[0].body
    if kind == "IntEnum":
        extra += ast.parse(INT_TEMPLATE.format(C=c)).body[0].body
    user, members = [], []
    last_int = 0
    seen = {}
    for st in cls.body:
        is_member = (isinstance(st, ast.Assign) and len(st.targets) == 1 and isinstance(st.targets[0], ast.Name)
                     and not st.targets[0].id.startswith("_"))
        if not is_member:
            user.append(st)
            continue
        name = st.targets[0].id
        v = st.value
        if isinstance(v, ast.Call) and enum_ref(v.func, names, modules) == "auto" and not v.args:
            v = ast.Constant(name.lower()) if kind == "StrEnum" else ast.Constant(last_int + 1)
        if isinstance(v, ast.Constant) and isinstance(v.value, int) and not isinstance(v.value, bool):
            last_int = v.value
        key = repr(v.value) if isinstance(v, ast.Constant) else None
        if key is not None and key in seen:
            # A second name for an existing value is an alias of that member.
            members.append((name, None, seen[key]))
            continue
        if key is not None:
            seen[key] = name
        members.append((name, v, None))
    body = [new.body[0]] + new.body[1:] + extra + user
    for name, v, alias in members:
        if alias:
            value = ast.parse(f"{c}.{alias}", mode="eval").body
        else:
            value = ast.Call(ast.Name(c, ast.Load()), [ast.Constant(name), v], [])
        body.append(ast.Assign([ast.Name(name, ast.Store())], value))
    listed = [n for n, _, alias in members if not alias]
    body.append(ast.parse(f"_pyrock_members_ = [{', '.join(f'{c}.{n}' for n in listed)}]").body[0])
    cls.bases = []
    cls.keywords = []
    cls.body = relocate(body, cls)
    return cls


def rewrite(tree):
    names, modules = enum_aliases(tree)
    if not names and not modules:
        return tree
    for st in tree.body:
        if isinstance(st, ast.ClassDef) and len(st.bases) == 1 and not st.keywords and not st.decorator_list:
            kind = enum_ref(st.bases[0], names, modules)
            has_init = any(isinstance(x, ast.FunctionDef) and x.name == "__init__" for x in st.body)
            if kind in ENUM_KINDS and not has_init:
                rewrite_enum(st, kind, names, modules)
    return tree


def main():
    path = sys.argv[1]
    with open(path, "rb") as f:
        src = f.read()
    try:
        tree = rewrite(ast.parse(src, filename=path))
    except SyntaxError as e:
        json.dump({"protocol": PROTOCOL, "error": {"msg": e.msg, "line": e.lineno or 0,
                                                  "col": e.offset or 0}}, sys.stdout)
        return
    json.dump({"protocol": PROTOCOL, "python": sys.version.split()[0], "module": conv(tree),
               "builtins": [n for n in dir(builtins) if not n.startswith("_")]},
              sys.stdout)


main()
