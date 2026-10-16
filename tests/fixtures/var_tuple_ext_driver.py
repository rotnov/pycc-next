# Drives `var_tuple_ext` (compiled with --ext, or the source in CPython):
# the output must be the same either way.
import sys

sys.path.insert(0, sys.argv[1])
import var_tuple_ext as m

k = m.parse_key("a.b.c")
print(k, type(k).__name__, m.parent(k), m.stem(k), m.count(("x", "y", "x"), "x"))
t = ("p", "q")
print(m.same(t) is t, m.join(t, ()) == t, m.join((), ()), m.described(t))
print(m.parent(m.parse_key("solo")), m.join(k, t))
p = m.Path(("root",))
print(p.child("leaf"), p.get(), p.get() == ("root",))
a, b, c = m.parse_key("x.y.z")
print(a, b, c)
try:
    m.stem(())
except IndexError as e:
    print("IndexError:", e)
