# Driver of tests/object_operand_pin.rs: imports `sys.argv[1]` (the
# compiled module or its CPython twin) with nesting armed, calls `probe`
# with nesting armed, then repeats `probe` without nesting and reports the
# live stub objects and the reference counts of two globals, which must
# not move.
import gc
import importlib
import sys

sys.path.insert(0, ".")
import object_operand_pin_stub as s

name = sys.argv[1]
s.NEST = name
m = importlib.import_module(name)
print("-- probe")
m.probe()
s.NEST = None
gc.collect()
live = s.LIVE

def counts():
    return [sys.getrefcount(m.get_h()), sys.getrefcount(m.get_d())]


before = counts()
for _ in range(5):
    m.probe()
gc.collect()
print("-- live", s.LIVE - live, "refs", *[a - b for a, b in zip(counts(), before)])
