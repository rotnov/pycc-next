# In-place set operators update the left operand itself: every alias sees
# the result, and dropping one alias leaves the others valid.


def words(prefix: str, n: int) -> list[str]:
    out: list[str] = []
    for i in range(n):
        out.append(prefix + str(i))
    return out


def main() -> None:
    a = set(words("w", 12))
    b = a
    a &= set(words("w", 6))
    print(len(a), len(b), sorted(b))
    del a
    b.add("late" + "comer")
    print(sorted(b))

    c = set(words("v", 4))
    d = c
    c |= set(words("u", 2))
    c -= set(words("v", 2))
    c ^= set(words("t", 2))
    del c
    print(sorted(d))
    d &= d
    d |= d
    print(len(d))
    d ^= d
    print(len(d), d)

    n: set[int] = {1, 2, 3, 4}
    m = n
    n &= {2, 4, 8}
    print(n, m)

    f = frozenset(words("f", 3))
    g = f
    f &= frozenset(words("f", 1))
    print(sorted(f), sorted(g))


main()
