# Drives `object_tuples` (compiled with --ext --non-strict, or the source in
# CPython): the output must be the same either way, including a list that
# came back inside a tuple and is mutated through its alias.
import sys

sys.path.insert(0, sys.argv[1])
import object_tuples as m

n, v = m.nested("[")
v[0].append(n)
print(v, v[0] is v[1])
pos, value = m.parse_value("[", 0)
value.append(pos)
print(pos, value)
k = m.parse_value("kq", 0)
print(k, type(k).__name__, type(k[1]).__name__)
print(m.pair("t"), m.pair("x"))
print(m.items(["f", "k", "12"]))
