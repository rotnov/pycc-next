"""`str()`, `repr()` and `bool()` of native values: a tuple, a set, a
frozenset, an optional and the containers CPython renders, and a value's
truth value."""

s = {3, 1, 2}
t = (1, "a", 2.5)
pair = (4, 5)
fs = frozenset([7])
d = {"a": [1, 2]}
o: int | None = None
nested = [(1, "x"), (2, "y")]
print(str(s), str(t), str(pair), str(fs), str(d), str(o), str(nested))
print(repr(t), repr(s), repr(pair), len(str(t)) + len(repr(fs)))
o = 5
print(str(o), "[" + str(pair) + "]")
print(bool(0), bool(3), bool(0.0), bool(""), bool("x"), bool([]), bool([0]), bool(), bool(pair))
flags = [bool(x % 2) for x in range(4)]
print(flags, sum(1 for f in flags if f))
