def read_value(d: int) -> int:
    try:
        x = 10 // d
    except ZeroDivisionError:
        pass
    return x
