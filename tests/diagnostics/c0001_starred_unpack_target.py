def f(xs: list[tuple[int, int, int]]) -> int:
    for a, *b in xs:
        return a
    return 0
