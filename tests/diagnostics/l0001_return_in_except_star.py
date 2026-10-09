def f() -> int:
    try:
        pass
    except* ValueError:
        return 1
    return 0
