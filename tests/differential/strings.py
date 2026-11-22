def greet(name, times):
    return ("hi " + name + "! ") * times


def pick(a, b):
    return a or b


def both(a, b):
    return a and b


s = "abc"
t = s
s += "def"
print(s, t, s < t, "b" > "abc", "a" == "a", "Z" < "a")
print(greet("bob", 2))
print(pick("", "fallback"), pick("first", "x"), both("", "x"), both("y", "z"))
print(pick(0, 5), pick(3, 5), both(0, 5), both(2, 5), pick(0.0, 1.5))
print("-" * 10, 3 * "ab", "x" * 0, "x" * -1)
print(f"{s}:{len(s)}:{1.5}:{True}:{None}")
print("tab\there", "quote's", 'dq"')
