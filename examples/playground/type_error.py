# expect: compile error (P301); CPython fails at run time instead
def label(count):
    return "items: " + count

print(label("three"))
print(label(3))   # CPython fails here when it runs; pyrock refuses to compile it
