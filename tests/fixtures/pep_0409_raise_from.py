def fail(kind: int) -> int:
    if kind == 0:
        raise ValueError("primary")
    return kind


def chained(kind: int) -> str:
    try:
        try:
            fail(kind)
        except ValueError:
            raise TypeError("chained") from KeyError("cause")
    except TypeError:
        return "chained caught"
    return "no chain"


def suppressed(kind: int) -> str:
    try:
        try:
            fail(kind)
        except ValueError:
            raise RuntimeError("suppressed") from None
    except RuntimeError:
        return "suppressed caught"
    return "no suppression"


print(chained(0))
print(chained(5))
print(suppressed(0))
print(suppressed(5))
