class G(ExceptionGroup):
    pass


def f() -> int:
    try:
        pass
    except* G:
        pass
    return 0
