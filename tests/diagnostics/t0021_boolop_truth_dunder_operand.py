class Empty:
    def __init__(self) -> None:
        self.n = 0

    def __bool__(self) -> bool:
        return False


def f(e: Empty) -> None:
    if e or True:
        print(1)
