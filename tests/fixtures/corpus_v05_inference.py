# v0.5 corpus round: signatures and empty containers strict mode now infers.

# Empty containers bound by one tuple assignment are typed by their own uses.
ys, xs, seen = [], [], set()
for i in range(5):
    xs.append(i * 3)
    ys.append(i * i)
    seen.add(i % 2)


def first_greater(arr, start, end, val):
    while start <= end:
        mid = (start + end) // 2
        if arr[mid] > val:
            end = mid - 1
        else:
            start = mid + 1
    return start


for i in range(len(ys)):
    print(first_greater(xs, 0, len(xs) - 1, ys[i]))
print(sorted(seen))

# A tuple assignment whose values read a target still evaluates every
# value before storing any.
a, b = 1, 2
a, b = b, a
print(a, b)
c = 5
c, d = 1, c
print(c, d)
e, f, g = [], 0, -1
e.append("x")
print(e, f, g)

# A dict value passed to an unannotated function.
d2 = {}
for k in range(4):
    d2[k] = k * 10


def plus_one(i):
    return i + 1


print(plus_one(d2[3]))


# A constructor argument read from an unannotated function's return, and
# one built inside a comprehension.
def pair():
    return map(int, "4 7".split())


class Node:
    def __init__(self, ident):
        self.ident = ident
        self.rank = 0

    def __repr__(self):
        return "Node({!r})".format(self.ident)


class Tools:
    def __init__(self, size, mod):
        size += 1
        self.size = size
        self.mod = mod

    def mul(self, n, r):
        return n * r % self.mod


n1, k1 = pair()
tools = Tools(n1, k1)
print(tools.size, tools.mul(5, 3))
nodes = [Node(i) for i in range(3)]
print(nodes[2].ident, repr(nodes[1]))

# `str.format` over a literal.
name = "pycc"
print("{} has {} letters".format(name, len(name)))
print("{1}-{0}-{1}".format("a", "b"))
print("{!r} {!s} {:>6} [{:<4}] {:.3f}".format(name, name, 42, "ab", 2.0 / 3))
print("{{literal}} {0}".format(1.5))
print("{!r:>8}|".format("q"))
print("{}".format([1, 2]), "".format(), "x".format())
print("{0}".format(True, 99))


# A function reading a global the module binds only inside a loop.
def scaled(xs):
    best = -1 * float("inf")
    for x in xs:
        best = max(best, x / size)
    return best


for size in (2, 4):
    print(scaled([1, 2, 3]))
