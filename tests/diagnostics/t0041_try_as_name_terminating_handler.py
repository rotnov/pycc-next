def f(d: int) -> int:
    try:
        e = 10 // d
    except ZeroDivisionError as e:
        raise
    return e
