# A package: its body runs once, before any of its submodules.
print("init _pkgapp")
VERSION = "1.2"

from . import models


def describe():
    return "pkgapp " + VERSION + " with " + models.KIND

try:
    from ._speedups import fast_total
except ImportError:
    fast_total = None

if VERSION:
    from .services import rules as _rules
    TAX = _rules.TAX
