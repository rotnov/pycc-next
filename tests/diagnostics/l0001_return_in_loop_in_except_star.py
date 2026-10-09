def f() -> int:
    try:
        pass
    except* ValueError:
        for i in range(3):
            return 1
    return 0
