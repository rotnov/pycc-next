class InvariantBroken(AssertionError):
    pass


def note(tag: str) -> str:
    print("evaluated", tag)
    return tag


def check(x: int) -> int:
    assert x > 0
    assert x > 1, "need more than one"
    return x * 2


def truthy(s: str, n: int, f: float) -> str:
    try:
        assert s, "str"
        assert n, "int"
        assert f, "float"
    except AssertionError as e:
        return f"failed on {e}"
    return "passed"


def main() -> None:
    print(check(5))
    try:
        check(1)
    except AssertionError as e:
        print("message:", e)
        print("again:", e)
    try:
        check(0)
    except Exception as e:
        print(f"empty [{e}]")
    assert True, note("passing")
    try:
        assert 1 > 2, note("failing")
    except AssertionError as e:
        print(f"caught {e}")
    print(truthy("x", 3, 0.5), truthy("", 3, 0.5), truthy("x", 0, 0.5), truthy("x", 3, 0.0))
    assert (y := 4) > 3
    print(y + 1)
    try:
        raise InvariantBroken("user subclass")
    except AssertionError as e:
        print(f"{e}")
    print(issubclass(AssertionError, Exception), issubclass(InvariantBroken, AssertionError))


main()
assert 2 > 1, "module level"
print("done")
