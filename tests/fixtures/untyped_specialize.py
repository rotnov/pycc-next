"""Untyped text and index code, built as an `--ext --non-strict` module.

Each function and method below loops over unannotated parameters that the
compiler guesses are `str` or `int` from how the body uses them, so it gets a
guarded native specialization; `untyped_specialize_driver.py` calls them with
the guessed types and with others (a `str` subclass, `bytes`, a `list`,
`None`, `bool`), and must print what CPython prints for the source module.
"""


class Text:
    def __init__(self):
        self.sep = "-"

    def common_prefix(self, a, b):
        n = min(len(a), len(b))
        i = 0
        while i < n and a[i] == b[i]:
            i += 1
        return i

    def common_suffix(self, a, b):
        i = 0
        while i < len(a) and i < len(b) and a[-i - 1] == b[-i - 1]:
            i += 1
        return a[len(a) - i:]

    def scan(self, text, start):
        out = []
        for i in range(start, len(text)):
            out.append(text[i])
        return self.sep.join(out)


def windows(s, k):
    res = []
    for i in range(len(s) - k + 1):
        res.append(s[i:i + k])
    return res


def stats(s, sub):
    vowels = 0
    for c in s:
        if c in "aeioué":
            vowels += 1
    return (
        vowels,
        s.find(sub),
        s.rfind(sub),
        s.count(sub),
        sub in s,
        s.startswith(sub),
        s.endswith(sub),
        s[::-1],
        s[1:-1:2],
        s[-3:],
        f"{s[:2]}|{len(s)}",
    )


def char_at(s, i):
    seen = 0
    while seen < 1:
        seen += 1
    return s[i]


def compare(a, b):
    for _ in range(1):
        pass
    return (a < b, a == b, a != b, a + b, a * 2, len(a + b))


def total(n):
    t = 0
    for i in range(n):
        t += i * 2
    return t


def split_words(text, sep):
    words = []
    start = 0
    while start <= len(text):
        end = text.find(sep, start)
        if end == -1:
            end = len(text)
        words.append(text[start:end])
        start = end + len(sep)
    return words
