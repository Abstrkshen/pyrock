# helper.py uses `try`, so it is imported through CPython (P220); its own
# imports must not leave unused statics (dead_code warnings) in main.rs.
import helper

print(helper.load('{"n": 3}'))
