# #1209 (Part 1 of #1018): augmented assignment to an attribute and to a
# subscript. The container is a bare name and the index a name or a literal,
# so the rewrite `target = target op value` reads them twice without an
# observable difference (docs/TYPE_SYSTEM.md, "Augmented assignment"). Only
# scalars are printed.


def announce(label: str, value: int) -> int:
    print(label)
    return value


class Counter:
    def __init__(self, start: int) -> None:
        self.n = start
        self.n += 1

    def bump(self, k: int) -> None:
        self.n += k
        self.n *= 2

    def bump_renamed(this, k: int) -> None:
        this.n -= k


class Prop:
    def __init__(self) -> None:
        self._p = 10

    @property
    def p(self) -> int:
        print("get")
        return self._p

    @p.setter
    def p(self, value: int) -> None:
        print("set")
        self._p = value


def maybe_bump(x: int | None) -> int | None:
    if x is not None:
        x += 1
    return x


c = Counter(1)
c.bump(3)
print(c.n)
c.bump_renamed(4)
print(c.n)
alias = c
alias.n += 100
print(c.n)

# The property is read, then the value is evaluated, then the setter runs.
pr = Prop()
pr.p += announce("value", 5)
print(pr.p)

d: dict[str, int] = {"a": 1, "b": 2}
k = "a"
d[k] += 1
d["b"] *= 5
d[k] -= announce("rhs", 10)
print(d["a"])
print(d["b"])

# The load raises `KeyError` before the value expression runs.
try:
    d["missing"] += announce("never printed", 1)
except KeyError:
    print("KeyError")


def per_function(d: dict[str, int], key: str) -> int:
    d[key] += 7
    d[key] //= 2
    return d[key]


print(per_function(d, "a"))

# An induction variable rebound in its own body does not change the
# iteration, for `range` and for a list alike.
for i in range(4):
    if i == 1:
        i += 5
    print(i)

xs = [1, 2, 3]
for v in xs:
    v += 100
    print(v)
print(len(xs))

r = maybe_bump(3)
if r is not None:
    print(r)
r = maybe_bump(None)
if r is None:
    print("none")
