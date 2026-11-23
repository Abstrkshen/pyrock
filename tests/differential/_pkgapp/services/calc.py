from ..models import Item
from .. import VERSION
from . import rules


def total(items):
    t = 0.0
    for it in items:
        t += rules.price_with_tax(it.price)
    return round(t, 2)


def cheapest(items):
    best = items[0]
    for it in items:
        if it.price < best.price:
            best = it
    return best


def make(name, price):
    return Item(name, price)


def version():
    return "calc for " + VERSION


def report(items):
    from ..compat import dumps, price_with_tax
    return dumps({"n": len(items), "first": price_with_tax(items[0].price)})
