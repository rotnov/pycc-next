"""v0.8 accept 4: `str` methods text-processing code runs most (replace,
find, count, `in`, strip, join, `+`) on ASCII and non-ASCII text, short and
long, with one-byte, multi-byte and empty needles."""


def ident(s: str) -> str:
    return s


def esc(s: str) -> str:
    s = s.replace("&", "&amp;")
    s = s.replace("<", "&lt;")
    s = s.replace(">", "&gt;")
    return s.replace('"', "&quot;")


def multi(s: str, old: str, new: str) -> str:
    return s.replace(old, new)


def finds(s: str, sub: str) -> str:
    return f"{s.find(sub)} {s.rfind(sub)} {s.count(sub)} {sub in s} {s.find(sub, 2)}"


def strips(s: str) -> str:
    return "[" + s.strip() + "|" + s.lstrip() + "|" + s.rstrip() + "|" + s.strip("x") + "]"


def joined(a: str, b: str, sep: str) -> str:
    return sep.join([a, b, a])


def cat(a: str, b: str) -> str:
    return a + b


def split_text(s: str, sep: str) -> str:
    return "/".join(s.split(sep))


TEXTS = [
    "",
    "a",
    "plain",
    'a&b<c>"d"',
    "x" * 40 + "&" + "y" * 40,
    "héllo wörld & ünïcode ≠ ascii, a bit longer than twenty-two bytes",
    "  padded  ",
    "xxpaddedxx",
    "€€€",
]
NEEDLES = ["", "a", "&", "ll", "ö", "wörld", "zz", "y" * 3]


def main() -> None:
    for t in TEXTS:
        print(repr(esc(t)), repr(ident(t)), repr(cat(t, t)))
        print(repr(strips(t)))
        for sub in NEEDLES:
            print(finds(t, sub))
            if sub:
                print(repr(split_text(t, sub)))
                print(repr(joined(t, sub, sub)))
        print(repr(multi(t, "", "-")), repr(multi(t, "l", "LL")), repr(multi(t, "ö", "o")))
    print(split_text("a,,b", ""))


main()
