# Open generators of imported modules are finalized at exit as at CPython's
# shutdown: after the main module; modules without reference cycles first
# (assignment order), then modules whose functions reference their globals
# (generators held directly first). The relative order of several modules
# with cycles depends on CPython's garbage-collector internals (it changes
# once .pyc files are cached), so this test has one.
import _fin_cyclic
import _fin_plain
from _fin_gen import gen

m = gen("main.m")
next(m)
print("main end", _fin_cyclic.current() is _fin_cyclic.v)
