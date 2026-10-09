def f() -> int:
    try:
        pass
    except* (ValueError, BaseExceptionGroup):
        pass
    return 0
