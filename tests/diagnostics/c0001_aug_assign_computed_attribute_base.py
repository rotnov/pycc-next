class Box:
    def __init__(self) -> None:
        self.n = 0


def make() -> Box:
    return Box()


make().n += 1
