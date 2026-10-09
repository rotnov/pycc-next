def f() -> None:
    while True:
        try:
            pass
        except* ValueError:
            try:
                pass
            finally:
                break
