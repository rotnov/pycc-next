# `python select_regions_calls.py DIR SCENARIO`: calls `m` from DIR, then,
# for `rebind`, `code`, `defaults`, `dispatch` and `pattern`, changes what
# the region's copies rely on (a member global, a function's `__code__` or
# `__defaults__`, a dispatch target, the pattern global) and calls again;
# `midrun-*` rebinds a member (or replaces the pattern global) while a
# region call runs; `traceback` shows where exceptions escaping the copies
# are located, and `identity` which `int` a copy hands back. Each scenario
# runs in a process of its own: a copy left once stays left.
import re
import sys
import traceback

sys.path.insert(0, sys.argv[1])
import m

scenario = sys.argv[2]


def show(label, f, *args):
    try:
        print(label, repr(f(*args)))
    except Exception as e:
        print(label, type(e).__name__, e)


DOC = "ab [1, cd, [22 , ef]] \\\\\\ [\\\\] 7"


class MyInt(int):
    pass


class Index:
    """A position that is no `int`, usable as one where CPython asks for
    `__index__`."""

    def __init__(self, v):
        self.v = v

    def __index__(self):
        return self.v

    def __add__(self, other):
        return Index(self.v + other)

    def __ge__(self, other):
        return self.v >= other

    def __lt__(self, other):
        return self.v < other


class Match:
    def __init__(self, pos):
        self.pos = pos

    def end(self):
        return self.pos + 2

    def group(self):
        return "custom"


class Fake:
    def match(self, s, pos):
        return Match(pos)


def frames(label, f, *args):
    """The frames (function, line) an exception escaping `f` passed."""
    try:
        f(*args)
    except Exception as e:
        tb = traceback.extract_tb(e.__traceback__)
        print(label, type(e).__name__, [(t.name, t.lineno) for t in tb])


def run():
    show("parse", m.parse, DOC)
    show("tails", m.tails, "  xyz!")
    show("skip-bool", m.skip, "  a", True)
    show("skip-sub", m.skip, " a", MyInt(0))
    show("skip-big", m.skip, "ab", 2 ** 70)
    show("skip-neg", m.skip, "a  ", -2)
    show("items-out", m.items, "1 2]", 0, 0)
    show("value-err", m.value, "[1, ?]", 0, 0)
    show("escapes-a", m.escapes, "\\\\x", 0, True)
    show("escapes-b", m.escapes, "\\\\\\\\x", 0, False)
    show("word-out", m.word, "abc1", 0)
    show("size", m.size, "abc", lambda t: "x")
    show("sizes", m.sizes, "ab")


if scenario == "plain":
    run()
elif scenario == "rebind":
    run()
    m.skip = m.other_skip
    run()
elif scenario == "code":
    run()
    m.skip.__code__ = m.other_skip.__code__
    run()
elif scenario == "defaults":
    run()
    m.skip.__defaults__ = (" a",)
    run()
elif scenario == "dispatch":
    run()
    m.esc_a = lambda s, pos: (pos + 1, "Z")
    run()
    m.esc_b = m.esc_a
    run()
elif scenario == "pattern":
    run()
    m.WORD = re.compile("[a-z]")
    run()
    m.WORD = Fake()
    run()
elif scenario == "deep":
    # Unbounded recursion through the copies raises `RecursionError`; the
    # depth it is raised at is not compared (CPython 3.12+ bounds C
    # recursion by the stack, not by `sys.getrecursionlimit()`).
    show("ok", m.parse, "[" * 100 + "]" * 100)
    depth = 200000
    show("deep", m.parse, "[" * depth + "]" * depth)
    show("after", m.parse, DOC)
elif scenario == "traceback":
    # Raised in a region entry's copy, and through a chain of copies.
    frames("entry", m.items, "1 2", 0, 0)
    frames("chain", m.parse, "[1, ?]")
    frames("value", m.value, "[1, ?]", 0, 0)
    frames("word", m.word, "1", 0)
elif scenario == "identity":
    # docs/DEVIATIONS.md: a position a copy returns as it came is a new
    # `int` outside CPython's small-int cache; inside it, the same object.
    x = 2 ** 40
    small = int("1000")
    print("identity", m.skip("ab", x) == x, m.skip("ab", x) is x, m.skip("ab", small) is small)
elif scenario.startswith("midrun-pattern"):
    # `m.WORD` replaced while `parse` runs: `word`'s generic arm returns
    # the new match's `end()`, which the copies that called it convert.
    end = {"midrun-pattern-int": MyInt, "midrun-pattern-index": Index}[scenario]

    class Ends(Match):
        def end(self):
            return end(self.pos.__index__() + 2)

    class Patterns:
        def match(self, s, pos):
            return Ends(pos)

    class Replacing(str):
        done = False

        def __getitem__(self, i):
            if not Replacing.done:
                Replacing.done = True
                m.WORD = Patterns()
            return str.__getitem__(self, i)

    show("midrun", m.parse, Replacing("ab cd 12"))
    show("after", m.parse, "ab cd 12")
elif scenario.startswith("midrun"):
    # `m.skip` rebound while `parse` runs, by the first read of its `str`
    # argument: the rest of that call takes the generic arms, which convert
    # the new function's result to the `int` the copy needs.
    wrap = {"midrun-int": MyInt, "midrun-index": Index}[scenario]
    old = m.skip

    class Rebinding(str):
        done = False

        def __getitem__(self, i):
            if not Rebinding.done:
                Rebinding.done = True
                m.skip = lambda s, pos, chars=" ": wrap(old(s, pos.__index__(), chars))
            return str.__getitem__(self, i)

    show("midrun", m.parse, Rebinding("ab cd 12"))
    show("after", m.parse, "ab cd 12")
