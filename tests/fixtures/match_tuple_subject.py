"""`match` on a tuple subject whose elements have different types: each
position of a sequence pattern is checked against its own element type."""

for v in [(1, "a"), (2, "b"), (0, ""), (7, "zz")]:
    match v:
        case (0, _):
            print("zero")
        case (n, "a" | "b") if n > 0:
            print("ab", n)
        case (n, s):
            print("pair", n, s[:2])

for p in [(1.5, True, "x"), (0.0, False, "yy"), (2.5, True, "")]:
    match p:
        case (0.0, _, s):
            print("origin", s)
        case (x, True, "x") | (x, True, ""):
            print("flagged", x)
        case _:
            print("other", p)

for q in [(1, 2, 3), (4, 5, 6), (7, 0, 9)]:
    match q:
        case (1, *rest):
            print("one then", rest)
        case (a, 0, c):
            print("zero middle", a + c)
        case (a, b, c):
            print("sum", a + b + c)
