def f(x: int | None) -> None:
    if x is not None and x > 0:
        print(x)
