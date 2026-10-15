# frozenset(str), the str-argument set predicates and membership in a set
# display, in a strict native build.
WS: frozenset[str] = frozenset(" \t")
KEY_CHARS: frozenset[str] = frozenset("abcdefghijklmnopqrstuvwxyz" "0123456789" "-_")
CTRL: frozenset[str] = frozenset([chr(i) for i in range(32)]) | frozenset(chr(127))
ILLEGAL: frozenset[str] = CTRL - frozenset("\t")


def classify(c: str) -> str:
    if c in {"#", ";"}:
        return "comment"
    if c not in {"=", ":"}:
        return "other"
    return "sep"


def scan(text: str) -> None:
    print(len(WS), len(KEY_CHARS), len(CTRL), len(ILLEGAL), "\t" in CTRL, "\t" in ILLEGAL)
    print(ILLEGAL.isdisjoint(text), KEY_CHARS.issuperset(text), frozenset("ab").issubset(text))
    print(len(frozenset("")), len(frozenset("aaa")), "a" in frozenset("abc"))
    print([classify(c) for c in text])


scan("ab-c=1;")
scan("x\x01y")
scan("")
print(KEY_CHARS.issuperset("é"), frozenset("é") <= frozenset("éa"))
