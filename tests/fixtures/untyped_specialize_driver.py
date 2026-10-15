"""Calls `untyped_specialize` (imported as `m`) with the types its guards
expect and with others, printing every result or exception."""
import sys

import m


class S(str):
    def __getitem__(self, i):
        return "X"

    def __len__(self):
        return 3


class I(int):
    def __index__(self):
        return 1


def show(label, f, *args):
    try:
        print(label, repr(f(*args)))
    except Exception as e:
        print(label, f"{type(e).__name__}: {e}")


TEXTS = ["", "a", "hello world", "héllo wörld", "日本語テキスト", "a\U0001f600b\U0001f600c", "ab" * 40]
OTHERS = [S("abcdef"), b"abc", ["a", "b"], None, ("a", "b")]

t = m.Text()
for a in TEXTS + OTHERS:
    for b in TEXTS[:4] + OTHERS[:2]:
        show("common_prefix", t.common_prefix, a, b)
        show("common_suffix", t.common_suffix, a, b)
for a in TEXTS + OTHERS:
    # (A `range()` bound beyond 2**61 on the object path is a documented
    # deviation, so the huge index goes to the native path only.)
    for k in (0, 1, 2, -1, True, I(5), 1.5, None) + ((2 ** 70,) if a in TEXTS else ()):
        if a is not None or isinstance(k, int):
            # (A `range()` of two failing objects is a documented deviation.)
            show("scan", t.scan, a, k)
        show("windows", m.windows, a, k)
        show("char_at", m.char_at, a, k)
    if a in TEXTS:
        show("char_at", m.char_at, a, -2 ** 70)
        show("char_at", m.char_at, a, 2 ** 62)
for a in TEXTS + OTHERS:
    for sub in ("", "l", "o w", "é", "\U0001f600", "ab", S("l"), b"a"):
        show("stats", m.stats, a, sub)
        if sub:
            show("split_words", m.split_words, a, sub)
for a in TEXTS + OTHERS:
    for b in TEXTS[:4] + [S("x"), 3]:
        show("compare", m.compare, a, b)
for n in (0, 1, 10, -3, True, I(4), 2.5, "3"):
    show("total", m.total, n)
# The guards leave a subclass's own behaviour in place.
print(t.common_prefix(S("q"), "qqq"), m.windows(S("zz"), 1))
sys.stdout.flush()
