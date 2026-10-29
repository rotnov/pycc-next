class Box[T]:
    def __init__(self, item: T) -> None:
        self.item = item

    def get(self) -> T:
        return self.item


def churn(n: int) -> int:
    total = 0
    for i in range(n):
        b = Box[str]("x" + str(i))
        s: str = b.get()
        total += len(s)
        c = Box[int](i * 100000000000000000000)
        v: int = c.get()
        total += v % 7
    return total
