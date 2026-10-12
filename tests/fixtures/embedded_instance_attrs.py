# A compiled class instance that reaches CPython code as an object keeps
# its attributes there (an embedded executable installs the descriptors).
class Node:
    def __init__(self, id: int, name: str):
        self.id = id
        self.name = name
        self.parent = self

    @property
    def label(self) -> str:
        return self.name + "#" + str(self.id)


def show(x: object) -> object:
    return x.label + " " + str(x.id) + " " + x.parent.name


def rename(x: object, name: object) -> None:
    x.name = name


a = Node(3, "a")
print(show(a))
rename(a, "b")
print(a.name, show(a))
print(show(Node(-1, "z")))
