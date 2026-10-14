import sys

sys.path.insert(0, sys.argv[1])
import textkit
from textkit import core, version

print(textkit.__version__, textkit.__packager__, textkit.PREFIX, textkit.__doc__)
print(core.__name__, version.__name__, core.PREFIX, core._count, version.__version__)
print(textkit.Differ is core.Differ, core.re.__name__)
d = textkit.Differ()
D = textkit.Differ
print(D.INSERT, D.DELETE, D.EQUAL, D.RATIO, D.NAME, d.INSERT, d.NAME)
print(d.common_prefix("abcd", "abxy"), d.common_prefix("", "x"))
print(d.split("hello", 2), d.split("hello", 0), d.split("hello", 9))
diffs = d.ops("abcd", "abxyz")
print(diffs)
print(d.normalize(diffs, "head"), diffs)
print(d.last_equal([(0, "a"), (1, "b"), (0, "cd"), (-1, "e")]), d.last_equal([]))
print(d.score(5, [1, 4, 9]), d.score(2.5, [2]))
print(d.blank_end("a\n\n"), d.blank_end("a\n"))
d.timeout = 0
d.margin = 7
print(d.timeout == 0, d.timeout < 1, d.margin)  # an int stored in a float attribute (docs/DEVIATIONS.md)
print([hex(ord(c)) for c in d.encode(20)][14:18])
print(d.parse("12"))
try:
    d.parse("x1")
except ValueError as e:
    print("ValueError", e)
p = textkit.Patch()
p.start = 3
p.length = 2
p.diffs.append((1, "add"))
p.diffs.append((0, "same"))
q = core.copy_patch(p)
q.diffs.append((-1, "gone"))
print(str(p), end="")
print(q, end="")
print(repr(q), [p], len(p.diffs), len(q.diffs))
print(core.bump(), core.bump())
