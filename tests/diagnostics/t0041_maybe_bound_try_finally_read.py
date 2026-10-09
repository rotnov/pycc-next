def read_value(d: int) -> int:
    try:
        x = 10 // d
    except ZeroDivisionError:
        x = -1
    finally:
        print(x)
    return x
