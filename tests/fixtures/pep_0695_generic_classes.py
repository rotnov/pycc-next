class Container[T]:
    def __init__(self, value: T) -> None:
        self.value = value

    def retrieve(self) -> T:
        return self.value

    def store(self, v: T) -> None:
        self.value = v


c = Container[int](1)
print(c.retrieve())
c.store(2)
print(c.retrieve())

d = Container[str]("hi")
print(d.retrieve())
d.store("bye")
print(d.retrieve())
