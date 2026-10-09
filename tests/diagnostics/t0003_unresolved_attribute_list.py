class Buffer:
    def __init__(self) -> None:
        self.xs = []

    def size(self) -> int:
        return len(self.xs)


print(Buffer().size())
