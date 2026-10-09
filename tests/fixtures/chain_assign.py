# #1213 (Part 5 of #1018): chained assignment `t1 = ... = tn = e`. The value
# is evaluated once, then assigned to each target left to right, and each
# target's own base and key are evaluated as that target is assigned
# (docs/TYPE_SYSTEM.md, "Chained assignment"). Only scalars are printed, and
# `x = y = None` stays at module level: two `None` locals in a function abort
# the program today (#1238).


def announce(label: str, value: int) -> int:
    print(label)
    return value


class Box:
    def __init__(self, v: int) -> None:
        self.x = self.y = v
        self.z = 0


class Prop:
    def __init__(self) -> None:
        self._p = 0

    @property
    def p(self) -> int:
        return self._p

    @p.setter
    def p(self, value: int) -> None:
        print("set p")
        self._p = value


def in_function() -> int:
    a = b = 0
    print(a + b)
    c = d = e = f = announce("once", 7)
    return c + d + e + f


# Module level, literal and None.
m = n = 5
p = q = None
print(m, n, p is None, q is None)

# A side-effecting value is evaluated exactly once.
r = s = announce("module once", 3)
print(r, s)
print(in_function())

# A non-empty list aliases: both names are the same list.
xs = ys = [1, 2]
xs.append(3)
print(len(ys))

# Attribute targets, a property setter, and `__init__`'s pre-scan.
box = Box(4)
print(box.x, box.y)
box.x = box.z = announce("attrs", 9)
print(box.x, box.z)
pr = Prop()
t = pr.p = announce("prop value", 2)
print(t, pr.p)

# A `dict[str, int]` subscript target mixed with a name and an attribute.
d: dict[str, int] = {"k": 0}
u = d["k"] = box.y = announce("mixed", 11)
print(u, d["k"], box.y)

# Four targets.
w1 = w2 = w3 = w4 = announce("four", 1)
print(w1 + w2 + w3 + w4)

# A chain inside a loop body.
total = 0
for i in range(3):
    lo = hi = i * 2
    total = total + lo + hi
print(total)

# A comprehension value.
evens = odds = [k for k in range(4)]
print(len(evens), len(odds))

# The ordering traps: a later target's key reads the name an earlier target
# just rebound.
k1 = 1
k1 = d[f"{k1}"] = 5
print(k1, d["5"])
k2 = 2
k2 = d[f"{k2}"] = announce("trap", 8)
print(k2, d["8"])
