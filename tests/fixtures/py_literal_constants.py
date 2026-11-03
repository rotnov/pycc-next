# `str` literals a module hands to CPython objects: the same text, the same
# interning and the same sharing of equal constants as CPython's.
import collections
import sys

q = collections.deque()
q.append("raw")
q.append("raw")
q.append("a b")
q.append("a b")
q.append("")
q.append("é")
print(q[0] is q[1], q[2] is q[3], q[0] is sys.intern("raw"))
print(list(q))
d = collections.OrderedDict()
d["key one"] = 1
d["key_two"] = 2
print(list(d), "key_two" in d, d.get("key one"))


def tag(x: int) -> object:
    od = collections.OrderedDict()
    od["n"] = x
    od["label text"] = x * 2
    return od


print(tag(3), tag(4))
# Texts of only NUL bytes: LLVM folds such an array to a zero initializer.
z = collections.deque()
z.append("\0")
z.append("\0\0")
z.append("\0")
print([len(s) for s in z], z[0] is z[2], z[1] == "\x00" * 2)
