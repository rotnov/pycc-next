# v0.9: a compiled class Python code subclasses. A compiled call of a method
# or property the Python subclass overrides runs the override
# (`pycc_mir::dispatch`'s Python arm); an instance compiled code holds keeps
# its Python object alive (`v09_pysub_host.py`).


class Base:
    def __init__(self, n: int) -> None:
        self.n = n

    def name(self) -> str:
        return "base"

    def size(self) -> int:
        return self.n

    @property
    def label(self) -> str:
        return "B" + str(self.n)

    def show(self) -> str:
        return self.name() + ":" + str(self.size()) + ":" + self.label


def show_it(b: Base) -> str:
    return b.show()


_held: list[Base] = []


def keep(b: Base) -> int:
    _held.append(b)
    return len(_held)


def held_show() -> str:
    return _held[0].show()


def release() -> None:
    _held.clear()


print(show_it(Base(1)))
