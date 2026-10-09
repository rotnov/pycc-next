# #1254 (Part 1 of #1214), D-250: list, set and dict comprehensions in any
# expression position -- a call argument, a `return` value, an annotated
# assignment's value, an f-string, a `while` test, a short-circuited operand,
# nested inside another comprehension, and inside `try`. Every container is
# reduced to a scalar before it is printed.
from typing import Protocol


class Sized(Protocol):
    def size(self) -> int:
        ...


class Box:
    def __init__(self, n: int) -> None:
        self.n = n

    def size(self) -> int:
        return self.n + 1


def measure(s: Sized) -> int:
    return s.size()


def identity[T](x: T) -> T:
    return x


def _twice(v):
    return v * 2


def total(xs: list[int]) -> int:
    s = 0
    for v in xs:
        s = s + v
    return s


def squares(n: int) -> list[int]:
    if n < 0:
        return [0]
    return [i * i for i in range(n)]


def trace(label: str, v: int) -> int:
    print(label)
    return v


def measured(k: int) -> int:
    ys = [measure(Box(v)) for v in range(k)]
    return len(ys) + ys[k - 1]


def after_control(n: int) -> int:
    k = 0
    if n > 1:
        k = 1
    while k < 3:
        k = k + 1
    try:
        k = k + 1
    except ZeroDivisionError:
        k = 0
    return len([i for i in range(n) if i > k - 5])


def add(a: int, b: int) -> int:
    return a + b


def guarded(z: int, xs: list[int]) -> int:
    try:
        return add(len([x for x in xs]), 1 // z)
    except ZeroDivisionError:
        return -1


def step(s: int) -> int:
    try:
        return len([x for x in range(0, 3, s)])
    except ValueError:
        return -2


def show_big(v: int) -> int:
    print(v)
    return 1


# A call argument, and a `return` value.
print(total([x * 2 for x in range(5)]))
print(len(squares(4)), total(squares(4)))

# An annotated assignment's value, then its elements one at a time.
evens: list[int] = [x for x in range(10) if x % 2 == 0]
print(len(evens), evens[0], evens[4])

# A set and a dict comprehension over a dict source, with a filter.
d = {"a": 1, "bb": 2, "ccc": 3}
print(len({k: 1 for k in d if k != "a"}))
print({k: d[k] * 10 for k in d}["bb"])
print(len({d[k] % 2 for k in d}))

# An f-string interpolation.
print(f"{len([z for z in range(7) if z % 2 == 0])} evens below 7")

# A `while` test, re-evaluated on every pass.
n = 0
while len([y for y in range(n)]) < 3:
    n = n + 1
print(n)

# An `if` test.
if len([y for y in range(n) if y > 0]) == 2:
    print("if-test taken")
if len({y for y in range(n) if y > 5}):
    print("not printed")
else:
    print("if-test else")

# Short-circuited operands: the comprehension on the right runs only when
# the left operand does not decide the result.
print(n > 5 and len([trace("ran", w) for w in range(2)]) > 0)
print(n > 1 and len([trace("ran", w) for w in range(2)]) > 0)

# Evaluation order: the iterable's operands, then each element in turn,
# before the enclosing call's next argument.
print(add(total([trace("elt", q) for q in range(trace("stop", 2))]), trace("after", 5)))

# The loop name does not leak: `x` keeps the value it had before.
x = 100
print(total([x for x in range(3)]), x)

# Big-int intermediates inside the element and the filter: each one is
# released inside its own iteration, and only small ints reach the list.
print(len([show_big(10 ** (r + 28)) for r in range(3)]))
print(len([r for r in range(3) if show_big(10 ** (r + 28)) == 1]))

# Nested comprehensions, and the inner one naming the outer variable.
print(total([total([a + b for a in range(b)]) for b in range(4)]))
print(len([len([c for c in range(r)]) for r in range(5) if r > 1]))
# The inner `range(x)` reads the outer `x`; the inner element its own.
print(total([len([x for x in range(x)]) for x in range(4)]))

# Generic, protocol and unannotated helpers in the element.
print(total([identity(v) for v in range(4)]))
print(total([measure(Box(v)) for v in range(3)]))
print(total([_twice(v) for v in range(3)]))

# The statement form with a protocol helper in the element, at module and
# function scope (it used to stop the compiler with an internal error).
zs = [measure(Box(v)) for v in range(3)]
print(len(zs), zs[2], measured(4))

# After `if`, `while` and `try` in one function.
print(after_control(6))

# A comprehension beside a raising sibling argument, and a raising step.
print(guarded(0, [1, 2]), guarded(1, [1, 2]))
print(step(0), step(1))
for r in range(2):
    print(step(0), guarded(0, [3]))


# A comprehension flowing into and out of unannotated private helpers.
def _listed(k):
    return [i for i in range(k)]


def _count_list(xs):
    return len(xs)


def _count_set(xs):
    return len(xs)


def _count_dict(xs):
    return len(xs)


d2 = {"a": 1, "b": 2}
print(len(_listed(3)), _count_list([i for i in range(4)]))
print(_count_set({i % 2 for i in range(5)}), _count_dict({s: d2[s] for s in d2}))
