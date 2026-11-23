def add(a, b):
    return a + b

def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)

# add is called with ints, floats and strings:
# pyrock compiles three Rust functions from it.
print(add(1, 2), add(1.5, 2.25), add("py", "rock"))
print(fib(25))
