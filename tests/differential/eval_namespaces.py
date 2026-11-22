# eval/exec with an explicit namespace run in CPython; a module that uses
# globals() or eval() without one is imported through CPython as a whole.
import _eval_helper

print(eval("1 + 2", {}))
ns = {"x": 5}
exec("y = x * 2", ns)
print(ns["y"], eval("x + y", ns))
print(eval("a * b", {}, {"a": 6, "b": 7}))
print(_eval_helper.names(), _eval_helper.calc("A + 41"))
