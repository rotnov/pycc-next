"""An empty `frozenset()` takes its element type from its uses: compared
with, combined with, or tested against a `frozenset[str]`, it is one."""


def main() -> None:
    a = frozenset("")
    b: frozenset[str] = frozenset()
    print(a == frozenset(), a == b, len(b), frozenset("ab") == frozenset("ba"))
    print(frozenset("") != frozenset(), frozenset("a") == frozenset())
    c = frozenset("x") | frozenset()
    d = frozenset("xy") & frozenset()
    print(sorted(c), len(d), frozenset() == frozenset(), "x" in frozenset())
    print("y" not in frozenset(), "x" in c)
    e = frozenset()
    print(e == a, len(e), len(e) == 0)
    f = frozenset()
    print(f == frozenset([1, 2]) - frozenset([1, 2]))


main()
