"""Native `str` indexing, comparison, length and reference counting.

Every operation here has an inline common case (`crate::str_inline`): a
small index into ASCII text, a one-byte comparison, a counted length and
an immortal string's counts. The texts cover both payloads (inline up to
22 bytes, heap beyond), ASCII and non-ASCII, and the empty string, so each
inline case and each fall-back to the runtime runs.
"""


def scan(s: str) -> str:
    out: list[str] = []
    for i in range(-len(s), len(s)):
        c: str = s[i]
        if c == "o":
            out.append("O")
        elif c < "a":
            out.append("_")
        else:
            out.append(c)
    return "".join(out)


def index_error(s: str, i: int) -> str:
    try:
        return s[i]
    except IndexError as e:
        return "IndexError: " + str(e)


def order(a: str, b: str) -> str:
    return str(a < b) + str(a == b) + str(a > b) + str(a <= b) + str(a != b)


def held(s: str) -> list[str]:
    kept: list[str] = []
    for i in range(len(s)):
        c: str = s[i]
        kept.append(c)
        kept.append(c)
    return kept


def main() -> None:
    short = "hello, world"
    long = "a longer text, past the inline payload: hello"
    accented = "héllo wörld 中"
    empty = ""
    for s in [short, long, accented, empty]:
        print(len(s), scan(s))
        for i in [0, 1, -1, len(s) - 1, -len(s), len(s), -len(s) - 1, 10**30, -(10**30)]:
            print(" ", i, index_error(s, i))
    pairs = [("a", "b"), ("b", "a"), ("a", "a"), ("ab", "a"), ("a", "ab"),
             ("é", "z"), ("z", "é"), ("é", "è"), ("", "a"), ("", "")]
    for a, b in pairs:
        print(repr(a), repr(b), order(a, b))
    c = short[0]
    print(order(c, short[0]), order(c, "h"), order(short[4], long[-1]))
    kept = held(long)
    print(len(kept), "".join(kept[::2]) == long)
    kept = held(accented)
    print(len(kept), "".join(kept[::2]) == accented)


main()
