class A:
    def __init__(self, x: int) -> None:
        self.x = x

    def __repr__(self) -> str:
        return "A(" + str(self.x) + ")"

    def __lt__(self, other: "A") -> bool:
        return self.x < other.x

    def __eq__(self, other: "A") -> bool:
        return self.x == other.x


class B(A):
    def __str__(self) -> str:
        return "B-" + str(self.x)


class Plain:
    def __init__(self, x: int) -> None:
        self.x = x


def main() -> None:
    a = A(1)
    b = B(2)
    print(a)
    print(b)
    print(str(a), repr(a), str(b), repr(b))
    print(f"{a} {b} {a!r} {b!r} {b!s}")
    s = "it's"
    print(repr(s), repr("a\nb\\"), repr('say "hi"'), repr(1), repr(2.5), repr(True))
    print(repr([1, 2]), repr(["x"]), f"{s!r} {3!r} {s!s}")
    print(a < A(2), A(2) < a, a == A(1), a != A(1), a != A(5))
    print(a < A(2) < A(3), a == A(1) == A(2))
    p = Plain(1)
    q = Plain(1)
    r = p
    print(p == q, p == r, p != q, p is r, p is q, p is not q)
    print(hash("abc") == hash("abc"), hash(1.5) == hash(1.5), hash((1, "a", 2.0)) == hash((1, "a", 2.0)))
    print(hash(2.0) == hash(2), hash(0.5))


main()
