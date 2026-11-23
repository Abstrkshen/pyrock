# Packages next to the program: `import pkg.sub`, `from pkg.sub import f`,
# `from pkg import sub`, relative imports inside the package.
import _pkgapp.services.calc
from _pkgapp import models
from _pkgapp.services.calc import total, make
import _pkgapp.services.rules as rules

items = [make("a", 10.0), models.Item("b", 2.5), _pkgapp.services.calc.make("c", 7.0)]
print(_pkgapp.describe(), _pkgapp.VERSION)
print(total(items), _pkgapp.services.calc.cheapest(items))
print(rules.TAX, _pkgapp.services.calc.version(), _pkgapp.models.KIND)
from _pkgapp.compat import Model, dumps
import _pkgapp.compat

print(Model("m", 1.0), dumps([1, 2]), _pkgapp.compat.dumps({"k": 1}), _pkgapp.compat.price_with_tax(10.0))
print(_pkgapp.services.calc.report(items))
print(_pkgapp.fast_total is None, _pkgapp.TAX)
