import json


def safe_div(a, b):
    try:
        return a // b
    except ZeroDivisionError:
        return -1


def parse(s):
    try:
        v = int(s)
    except ValueError as e:
        print("bad:", e)
        return 0
    else:
        print("ok:", v)
        return v
    finally:
        print("finally", s)


def first_negative(n):
    i = 0
    while i < n:
        try:
            if i == 3:
                break
            if i % 2 == 0:
                continue
            print("odd", i)
        finally:
            print("step", i)
            i += 1
    return i


def lookup(key):
    if key != "a":
        raise KeyError(key)
    return 1


def classify(x):
    try:
        if x == 0:
            raise ValueError("zero")
        if x == 1:
            raise TypeError
        if x == 2:
            return lookup("b")
        if x == 3:
            return 10 // (x - 3)
        return x
    except (ValueError, TypeError) as e:
        print("value/type:", repr(e))
        return -1
    except LookupError as e:
        print("lookup:", e)
        return -2
    except ArithmeticError:
        print("arith")
        return -3


def reraise(x):
    try:
        try:
            return 100 // x
        except ZeroDivisionError:
            print("inner")
            raise
    except Exception as e:
        print("outer caught", e)
        return 0


def from_python(text):
    try:
        return json.loads(text)["n"]
    except json.JSONDecodeError:
        print("json error")
        return -1
    except KeyError as e:
        print("missing", e)
        return -2


def checked(x):
    assert x > 0, "x must be positive"
    return x


def finally_overrides():
    try:
        return 1
    finally:
        print("cleanup runs before return")


print(safe_div(7, 2), safe_div(1, 0))
print(parse("42"), parse("4x2"))
print(first_negative(10))
for v in range(5):
    print(classify(v))
print(reraise(0), reraise(5))
print(from_python('{"n": 3}'), from_python("{"), from_python('{"m": 1}'))
print(checked(5))
try:
    checked(-1)
except AssertionError as e:
    print("assert:", e)
print(finally_overrides())
err = ValueError("made, not raised")
print(err)
raise RuntimeError("uncaught at the end")
