import json

def total(prices):
    s = 0.0
    for p in prices:
        s += p
    return s

# json is a CPython module: these lines are boundaries (see Explain).
# total() gets a native list of floats and compiles to plain Rust.
order = json.loads('{"items": [{"price": 9.5}, {"price": 3.25}]}')
prices = [float(item["price"]) for item in order["items"]]
print(total(prices))

# Switch to native-only: every boundary becomes an error.
