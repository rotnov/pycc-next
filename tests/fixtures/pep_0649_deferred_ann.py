class Node:
    def __init__(self, value: int) -> None:
        self.value = value

    def update(self, other: Node) -> None:
        self.value = other.value

    def same(self, other: Node) -> bool:
        return self.value == other.value


a = Node(1)
b = Node(2)
a.update(b)
print(a.value)
print(a.same(b))
