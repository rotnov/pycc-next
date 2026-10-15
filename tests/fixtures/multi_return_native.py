"""Functions returning native tuples (`tuple[int, str]`,
`tuple[int, float, bool]`), unpacked by their callers: compiled code
returns the values without building the tuple. Results, evaluation order
and exceptions are CPython's; every case prints what CPython prints."""


def note(label: str, value: int) -> int:
    print("eval", label)
    return value


def word(src: str, pos: int) -> tuple[int, str]:
    start = pos
    while pos < len(src) and src[pos] != " ":
        pos += 1
    return pos + 1, src[start:pos]


def quoted(src: str, pos: int) -> tuple[int, str]:
    if src[pos] == "'":
        end = src.index("'", pos + 1)
        return end + 2, src[pos + 1 : end]
    return word(src, pos)


def forward(src: str, pos: int) -> tuple[int, str]:
    pos += 0
    return quoted(src, pos)


def stats(values: list[float]) -> tuple[int, float, bool]:
    if not values:
        return 0, 0.0, False
    total = 0.0
    for v in values:
        total += v
    return len(values), total / len(values), total > 10.0


def ordered(a: int, b: int) -> tuple[int, int]:
    return note("first", a), note("second", b)


def big(n: int) -> tuple[int, int]:
    return n, n * 10**25


def checked(src: str, pos: int) -> tuple[int, str]:
    if pos < 0:
        raise ValueError("negative position")
    return pos + 1, src[pos]


def main() -> None:
    src = "alpha 'two words' gamma"
    pos = 0
    while pos < len(src):
        pos, item = forward(src, pos)
        print(pos, item)
    n, mean, large = stats([1.5, 2.5, 9.0])
    print(n, mean, large)
    n, mean, large = stats([])
    print(n, mean, large)
    a, b = ordered(1, 2)
    print(a, b)
    small, huge = big(7)
    print(small, huge)
    at = 0
    text = "unset"
    try:
        at, text = checked("xy", -1)
    except ValueError as e:
        print("ValueError", e, at, text)
    at, text = checked("xy", 1)
    print(at, text)
    kept = word("solo", 0)
    print(kept)


main()
