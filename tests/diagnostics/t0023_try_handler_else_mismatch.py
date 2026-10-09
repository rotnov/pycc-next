def pick(d: int) -> None:
    try:
        y = 10 // d
    except ZeroDivisionError:
        x = "s"
    else:
        x = 1
    print(x)
