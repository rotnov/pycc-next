def read_error() -> None:
    try:
        raise ValueError("v")
    except ValueError as e:
        pass
    print(e)
