# Issue #1: a module-level function name bound by its `def` and by other
# bindings. Compiled callers must see every rebinding, in program order, as
# CPython does. Built with --non-strict (natively and as an --ext module).
import os.path
import sys


# A `def` replaced by a `def` in a module-level block (runs in CPython).
def twice(x):
    return x + 1


def use_twice(v):
    return twice(v)


print("redef in block:", use_twice(1))
if sys.version_info >= (3, 0):
    def twice(x):
        return (lambda: x * 10)()
print("redef in block:", use_twice(2))


# A `def` rebound to a lambda, to a CPython callable and to a class.
def scale(x):
    return x + 1


def use_scale(v):
    return scale(v)


print("lambda:", use_scale(1))
scale = lambda x: x * 100
print("lambda:", use_scale(2))


def base(p):
    return p + "!"


def use_base(p):
    return base(p)


print("cpython callable:", use_base("a/b"))
base = os.path.basename
print("cpython callable:", use_base("a/b"))


class Box:
    def __init__(self, x):
        self.x = x


def make(x):
    return x + 1


def use_make(v):
    return make(v)


print("class:", use_make(1))
make = Box
print("class:", use_make(2).x)


# Rebound inside a function through `global`.
def step(x):
    return x + 1


def use_step(v):
    return step(v)


def rebind_step():
    global step
    step = lambda x: x * 7


print("global:", use_step(1))
rebind_step()
print("global:", use_step(2))


# Rebound to another compiled function, then defined again.
def plus_one(x):
    return x + 1


def pick(x):
    return x * 2


def use_pick(v):
    return pick(v)


print("other def:", use_pick(3))
pick = plus_one
print("other def:", use_pick(3), pick(3))


def pick(x):
    return x * 3


print("other def:", use_pick(3), pick(3))


# Bound first by a value, then by a `def`.
first = plus_one


def use_first(v):
    return first(v)


print("value first:", use_first(3))


def first(x):
    return x * 5


print("value first:", use_first(3))

# Rebound by a loop and a conditional assignment.
def loop_var(x):
    return x


def use_loop_var(v):
    return loop_var(v)


for loop_var in (abs, str):
    print("loop:", use_loop_var(-4))


def cond(x):
    return x + 1


if len(sys.argv) >= 0:
    cond = lambda x: x - 1


def use_cond(v):
    return cond(v)


print("conditional:", use_cond(10))


# A function used as a value, then defined again: the value is the first.
def early(x):
    return x + 1


alias = early


def early(x):
    return x * 2


def use_both(v):
    return early(v) + alias(v)


print("alias:", alias(3), early(3), use_both(3))


# A `def` that runs in CPython, called by compiled code before and after.
def use_late(v):
    try:
        return late(v)
    except NameError as e:
        return str(e)


print("late:", use_late(1))
if os.sep:
    def late(x):
        return (lambda: x + 5)()
print("late:", use_late(2))
