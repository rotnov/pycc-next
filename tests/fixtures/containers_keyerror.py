"""A missing key in a dict whose values are struct tuples or lists raises
`KeyError` before anything reads the (absent) value; so do `pop` and an
empty-set `pop`."""


def lookups() -> None:
    pairs: dict[str, tuple[int, int]] = {"a": (1, 2)}
    lists: dict[int, list[int]] = {1: [3, 4]}
    print(pairs["a"], lists[1])
    try:
        print(pairs["zz"])
    except KeyError as e:
        print("pairs", e)
    try:
        print(lists[7])
    except KeyError as e:
        print("lists", e)
    try:
        print(pairs.pop("zz"))
    except KeyError as e:
        print("pop", e)
    try:
        x, y = pairs["q"]
        print(x, y)
    except KeyError as e:
        print("unpack", e)
    s: set[tuple[int, int]] = set()
    try:
        print(s.pop())
    except KeyError as e:
        print("set pop", e)
    print(lists[2])


lookups()
