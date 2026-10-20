"""An exception class with a constructor of its own, raised and caught by compiled
code (`tests/exception_constructor.rs`)."""


class MissingRef(Exception):
    def __init__(self, ref: str) -> None:
        self.ref = ref
        super().__init__(f"Missing definition for ref {self.ref!r}")


def find(table: dict, key: str) -> int:
    if key not in table:
        raise MissingRef(key)
    return table[key]


def lookup_len(table: dict, key: str) -> int:
    try:
        return find(table, key)
    except MissingRef as e:
        return len(e.ref)


def kinds(o) -> str:
    return f"{isinstance(o, ValueError)} {isinstance(o, KeyError)} {isinstance(o, Exception)}"


def main() -> None:
    print(find({"a": 1}, "a"), lookup_len({}, "abc"), lookup_len({"x": 7}, "x"))
    try:
        find({}, "zz")
    except MissingRef as e:
        print(type(e).__name__, e, e.ref, isinstance(e, Exception), kinds(e))
    print(kinds(KeyError("k")), kinds(ValueError()), kinds(3))


if __name__ == "__main__":
    main()
