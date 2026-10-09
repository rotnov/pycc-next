# #1266 (Part 5 of #1218): a value-less class-body annotation declares the
# type of an instance attribute that `__init__` assigns. An empty `[]`/`{}`
# establishing a declared container slot takes the declared type, and a
# declaration may sit in a subclass. Only scalars are printed: printing a
# container is a separate gap (#1018). A type-parameter declaration is
# proved in `tests/issue_1266_class_body_declarations.rs` instead: a
# generic class beside a `super()` call trips an unrelated refusal.

class Counter:
    name: str
    total: float
    hits: int
    seen: bool
    counts: dict[str, int]
    order: list[int]

    def __init__(self, name: str) -> None:
        self.name = name
        self.total = 0.0
        self.hits = 0
        self.seen = False
        self.counts = {}
        self.order = []

    def add(self, key: str, v: int) -> None:
        # A store through the slot itself is #891; a local alias is the same
        # dict object.
        m = self.counts
        m[key] = m.get(key, 0) + v
        self.order.append(v)
        self.total = self.total + v / 2
        self.hits = self.hits + 1
        self.seen = True

    def reset(self) -> None:
        self.counts = {}
        self.order = []


class Tagged(Counter):
    tag: str

    def __init__(self, name: str) -> None:
        super().__init__(name)
        self.tag = name
        self.tag = self.tag + "!"


c = Counter("a")
d = Counter("b")
c.add("x", 3)
c.add("x", 4)
c.add("y", 1)
d.add("z", 5)
print(c.name, c.hits, c.total, c.seen, len(c.counts), c.counts["x"], len(c.order))
print(d.name, d.hits, d.total, len(d.counts), c.order[0] + d.order[0])
c.reset()
print(len(c.counts), len(c.order), c.counts.get("x", -1))
t = Tagged("t")
t.add("k", 2)
print(t.tag, t.hits, t.counts["k"])
