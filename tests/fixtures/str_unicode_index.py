"""Indexing, slicing, `len`, search and membership on `str`, ASCII and not:
pycc's `str` is UTF-8 and indexes by code point, with the count and an
offset table cached per string."""


def show_all(s: str) -> None:
    n = len(s)
    print(n, repr(s))
    for i in range(-n, n):
        print(i, s[i], end=" ")
    print()
    print(repr(s[1:]), repr(s[:-1]), repr(s[2:5]), repr(s[::-1]), repr(s[::2]), repr(s[1::3]))
    print(repr(s[-3:-1]), repr(s[5:2]), repr(s[100:]), repr(s[-100:2]), repr(s[3:3]))
    print(s.find("é"), s.rfind("o"), s.find("xyz"), s.count("o"), "o w" in s, "\U0001f600" in s)
    total = 0
    for c in s:
        if c == "o":
            total += 1
    print(total, s.startswith(s[:3]), s.endswith(s[-2:]), s[0] == s[n - n], s[n - 1] < "~")


def long_text(k: int) -> str:
    parts: list[str] = []
    for i in range(k):
        parts.append("abé日" if i % 3 == 0 else "xyz")
    return "".join(parts)


def compare_chars(a: str, b: str) -> int:
    same = 0
    i = 0
    while i < len(a) and i < len(b):
        if a[i] == b[i]:
            same += 1
        i += 1
    return same


def at(s: str, i: int) -> str:
    return s[i]


for text in ["hello world", "héllo wörld", "日本語テキスト ok", "a\U0001f600b\U0001f600c\U0001f600 o"]:
    show_all(text)

big = long_text(500)
print(len(big), big[0], big[1499], big[-1], big[700:712], big.find("日"), big.rfind("日"))
print(compare_chars(big, big[::-1]), compare_chars(big, big[1:] + "q"), compare_chars(big[10:], big[10:]))
print(repr(big[1003:997:-1]), repr(big[-5:]), len(big[::7]))

def try_at(s: str, i: int) -> None:
    try:
        print(at(s, i))
    except IndexError as e:
        print("IndexError:", e)


for i in [0, 3, -1, -4, 4, -5]:
    try_at("été!", i)
try_at("été!", 2 ** 62)
try_at("abc", 2 ** 63 - 1)
try_at("été!", 2 ** 70)
try_at("abc", -2 ** 70)
print(at("\U0001f600", 2 ** 70))
