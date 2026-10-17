"""`int | float` containers looked up with a `float` (v0.6 accept 3): a
membership test of a list, a dict and a set, a dict read and `.get()`,
with floats too large or too small to be held inline (a boxed float)
beside ordinary ones, and `float` keys and elements stored. A lookup only
borrows its needle; `tests/num_union.rs` also probes that the boxed floats
converted for it are released."""


def num(x: int | float) -> int | float:
    return x


def lookups(n: int) -> int:
    xs: list[int | float] = [1, 2.5]
    xs.append(3)
    d: dict[int | float, int] = {}
    d[1] = 10
    d[2.5] = 20
    s = {num(4), num(0.5)}
    hits = 0
    for i in range(n):
        big = 1e100 * (i + 1)
        tiny = 1e-100 / (i + 1)
        for needle in (big, tiny, 2.5, 3.0, 1.0, 0.25, 0.75, -big):
            if needle in xs:
                hits += 1
            if needle not in d:
                hits += 2
            if needle in s:
                hits += 4
            hits += d.get(needle, 1000)
        try:
            print(d[big])
        except KeyError as e:
            print("KeyError", e)
        print(d[2.5], d[1.0], tiny in s, big not in xs)
        d[0.25] = i
        s.add(0.75)
    return hits


def main() -> None:
    print(lookups(3))


main()
