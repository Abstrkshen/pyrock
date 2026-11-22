# Builtins that read the caller's frame have nothing to read in compiled code.
print(eval("1 + 2"))
exec("y = 1")
print(len(globals()))
print(locals())
print(vars())
print(dir())
