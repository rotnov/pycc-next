"""Core text operations of textkit."""

import re

PREFIX = "tk"
_count = 0


class Differ:
    """Class constants, a call-initialized class attribute, untyped methods."""

    INSERT = 1
    DELETE = -1
    EQUAL = 0
    RATIO = 0.5
    NAME = "differ"
    BLANK = re.compile(r"\n\r?\n$")

    def __init__(self):
        self.timeout = 1.0
        self.margin = 4

    def common_prefix(self, a, b):
        n = min(len(a), len(b))
        for i in range(n):
            if a[i] != b[i]:
                return i
        return n

    def split(self, text, at):
        # Returns a tuple, or None when there is nothing to split.
        def halves(t, i):
            if i <= 0 or i >= len(t):
                return None
            return (t[:i], t[i:])

        found = halves(text, at)
        if not found:
            return None
        return found

    def ops(self, a, b):
        diffs = []
        k = self.common_prefix(a, b)
        if k:
            diffs.append((self.EQUAL, a[:k]))
        if a[k:]:
            diffs.append((self.DELETE, a[k:]))
        if b[k:]:
            diffs.append((self.INSERT, b[k:]))
        return diffs

    def normalize(self, diffs, head):
        # Slice assignment on an object list, a method with no `return`.
        diffs[:0] = [(self.EQUAL, head)]
        diffs[len(diffs) - 1:] = []

    def last_equal(self, diffs):
        last = None
        i = 0
        while i < len(diffs):
            if diffs[i][0] == self.EQUAL:
                last = diffs[i][1]
            elif last and len(last) > 1:
                return last
            i += 1
        return last

    def score(self, loc, xs):
        # A nested helper called with an object and with an int.
        def dist(x):
            return abs(loc - x) / 10.0

        best = dist(0)
        for x in xs:
            best = min(best, dist(x))
        return best

    def blank_end(self, text):
        return bool(self.BLANK.search(text))

    def encode(self, n):
        return "".join(chr(0xD7F0 + i) for i in range(n))

    def parse(self, token):
        try:
            n = int(token)
        except ValueError:
            raise ValueError("bad number: " + token)
        return n


class Patch:
    def __init__(self):
        self.diffs = []
        self.start = None
        self.length = 0

    def __str__(self):
        text = ["@@ ", str(self.start), ",", str(self.length), " @@\n"]
        signs = {1: "+", -1: "-", 0: " "}
        for op, data in self.diffs:
            text.append(signs[op] + data + "\n")
        return "".join(text)

    def __repr__(self):
        return "Patch(" + str(self.start) + ")"


def copy_patch(p):
    q = Patch()
    q.diffs = p.diffs[:]
    q.start = p.start
    q.length = p.length
    return q


def bump():
    global _count
    _count += 1
    return _count
