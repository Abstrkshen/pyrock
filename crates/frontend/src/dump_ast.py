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


def cached_property_names(tree):
    """Local names of functools.cached_property, and of the functools module."""
    names, modules = set(), set()
    for st in tree.body:
        if isinstance(st, ast.ImportFrom) and st.module == "functools" and st.level == 0:
            for a in st.names:
                if a.name == "cached_property":
                    names.add(a.asname or a.name)
        elif isinstance(st, ast.Import):
            for a in st.names:
                if a.name == "functools":
                    modules.add(a.asname or "functools")
    return names, modules


CACHED_TEMPLATE = """
class _M:
    @property
    def {N}(self):
        try:
            return self._pyrock_cached_{N}
        except AttributeError:
            v = self._pyrock_compute_{N}()
            self._pyrock_cached_{N} = v
            return v
"""


def rewrite_cached_properties(tree):
    """`@cached_property def f(self)` as a property that computes once per
    instance and stores the value in an instance attribute (reading it before
    it is set raises AttributeError, which starts the computation)."""
    names, modules = cached_property_names(tree)
    if not names and not modules:
        return

    def is_cached(d):
        return (isinstance(d, ast.Name) and d.id in names) or (
            isinstance(d, ast.Attribute) and d.attr == "cached_property" and isinstance(d.value, ast.Name) and d.value.id in modules)

    for cls in [n for n in ast.walk(tree) if isinstance(n, ast.ClassDef)]:
        body = []
        for st in cls.body:
            if isinstance(st, ast.FunctionDef) and len(st.decorator_list) == 1 and is_cached(st.decorator_list[0]):
                prop = ast.parse(CACHED_TEMPLATE.format(N=st.name)).body[0].body[0]
                st.name = "_pyrock_compute_" + st.name
                st.decorator_list = []
                body += relocate([prop], st) + [st]
            else:
                body.append(st)
        cls.body = body


def namedtuple_names(tree):
    names, modules = set(), set()
    for st in tree.body:
        if isinstance(st, ast.ImportFrom) and st.module == "typing" and st.level == 0:
            for a in st.names:
                if a.name == "NamedTuple":
                    names.add(a.asname or a.name)
        elif isinstance(st, ast.Import):
            for a in st.names:
                if a.name == "typing":
                    modules.add(a.asname or "typing")
    return names, modules


def rewrite_namedtuples(tree):
    """`class P(NamedTuple)` as a frozen, ordered dataclass that also behaves
    like its tuple: len, indexing, iteration (unpacking), _asdict, _fields."""
    names, modules = namedtuple_names(tree)
    if not names and not modules:
        return

    def is_nt(b):
        return (isinstance(b, ast.Name) and b.id in names) or (
            isinstance(b, ast.Attribute) and b.attr == "NamedTuple" and isinstance(b.value, ast.Name) and b.value.id in modules)

    for cls in tree.body:
        if not (isinstance(cls, ast.ClassDef) and len(cls.bases) == 1 and is_nt(cls.bases[0]) and not cls.keywords and not cls.decorator_list):
            continue
        fields = [st.target.id for st in cls.body if isinstance(st, ast.AnnAssign) and isinstance(st.target, ast.Name)]
        getitem = "".join(f"        if i == {k}:\n            return self.{f}\n" for k, f in enumerate(fields))
        iters = "".join(f"        yield self.{f}\n" for f in fields) or "        return\n        yield\n"
        extra = ast.parse(f"""
class _M:
    _fields = {tuple(fields)!r}
    def __len__(self):
        return {len(fields)}
    def __getitem__(self, i):
        if i < 0:
            i += {len(fields)}
{getitem}        raise IndexError("tuple index out of range")
    def __iter__(self):
{iters}
    def _asdict(self):
        return {{{", ".join(f"{f!r}: self.{f}" for f in fields)}}}
""").body[0].body
        dec = ast.parse("dataclass(frozen=True, order=True)", mode="eval").body
        cls.bases = []
        cls.decorator_list = relocate([dec], cls)
        cls.body = cls.body + relocate(extra, cls)


TYPING_MODULES = ("typing", "typing_extensions")


def typing_imports(tree):
    """Local names bound to typing members (name -> member), and typing module aliases."""
    names, modules = {}, set()
    for st in tree.body:
        if isinstance(st, ast.ImportFrom) and st.module in TYPING_MODULES and st.level == 0:
            for a in st.names:
                names[a.asname or a.name] = a.name
        elif isinstance(st, ast.Import):
            for a in st.names:
                if a.name in TYPING_MODULES:
                    modules.add(a.asname or a.name)
    return names, modules


def typing_ref(node, names, modules):
    """The typing member a node names (`Protocol`, `typing.Generic`, ...)."""
    if isinstance(node, ast.Subscript):
        node = node.value
    if isinstance(node, ast.Name):
        return names.get(node.id)
    if isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name) and node.value.id in modules:
        return node.attr
    return None


class TypedDictCalls(ast.NodeTransformer):
    def __init__(self, classes):
        self.classes = classes

    def visit_Call(self, node):
        self.generic_visit(node)
        if isinstance(node.func, ast.Name) and node.func.id in self.classes and not node.args and all(k.arg for k in node.keywords):
            return ast.copy_location(ast.Dict([ast.Constant(k.arg) for k in node.keywords], [k.value for k in node.keywords]), node)
        return node


def rewrite_typing(tree):
    """Constructs that only matter to type checkers: `Protocol` / `Generic[T]`
    bases are dropped, TypeVar-like assignments removed, `NewType` is an
    identity function, and a TypedDict class is a plain dict (its calls with
    keyword arguments become dict literals)."""
    names, modules = typing_imports(tree)
    if not names and not modules:
        return
    typed_dicts = set()
    body = []
    for st in tree.body:
        if isinstance(st, ast.ClassDef):
            kinds = [typing_ref(b, names, modules) for b in st.bases]
            if "TypedDict" in kinds and not st.keywords:
                typed_dicts.add(st.name)
                continue
            st.bases = [b for b, k in zip(st.bases, kinds) if k not in ("Protocol", "Generic")]
        if isinstance(st, ast.Assign) and len(st.targets) == 1 and isinstance(st.targets[0], ast.Name) and isinstance(st.value, ast.Call):
            k = typing_ref(st.value.func, names, modules)
            if k in ("TypeVar", "ParamSpec", "TypeVarTuple"):
                continue
            if k == "NewType":
                f = ast.parse(f"def {st.targets[0].id}(x):\n    return x").body[0]
                body += relocate([f], st)
                continue
        body.append(st)
    tree.body = body
    if typed_dicts:
        TypedDictCalls(typed_dicts).visit(tree)


def rewrite(tree):
    rewrite_typing(tree)
    rewrite_namedtuples(tree)
    rewrite_cached_properties(tree)
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
