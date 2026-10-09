def f(c: bool) -> None:
    try:
        if c:
            e = 1
    except ZeroDivisionError as e:
        pass
    e = 5
    print(e)
