from typing import Any, Dict, List, MutableMapping, Optional, Tuple


class State:
    def __init__(self, env: MutableMapping[str, Any], depth: int = 0):
        self.env = env
        self.tokens: List[Dict[str, Any]] = []
        self.in_link: bool = False
        self.depth: int = depth
        self.limit: int = depth * 2 + 1
        self.name: str = "s" + str(depth)
        self.ratio: float = depth / 4
        self.parent: Optional[Any] = None
        self.ranges: Dict[int, Tuple[str, List[int]]] = {}
        self.counts: Dict[str, int] = {}
        self.marks: List[int] = []

    def push(self, kind: str, text: str) -> int:
        self.tokens.append({"type": kind, "text": text})
        self.counts[kind] = self.counts.get(kind, 0) + 1
        self.marks.append(len(text))
        return len(self.tokens)

    def child(self) -> "State":
        state = State(self.env, self.depth + 1)
        state.parent = self
        if self.depth > 2:
            state.limit: int = 7
        return state


def describe(state: State) -> str:
    return "%s depth=%d limit=%d ratio=%.2f link=%s tokens=%d counts=%s marks=%s" % (
        state.name,
        state.depth,
        state.limit,
        state.ratio,
        state.in_link,
        len(state.tokens),
        str(sorted(state.counts.items())),
        str(state.marks),
    )


class Box:
    def __init__(self, v: int) -> None:
        self.v: int = v


def scaled(n: int) -> int:
    total = 0
    for i in range(n):
        box = Box(i)
        if i % 2:
            box.v: int = box.v * 3
        total += box.v
    return total


root = State({"ref_links": {}})
root.push("text", "hello")
root.push("text", "world!")
root.push("emphasis", "x")
print(describe(root))
c = root.child()
print(describe(c), c.parent.depth, len(c.env))
for _ in range(3):
    c = c.child()
print(describe(c))
print(scaled(5))
print(describe(root.child()))
print(Box(2).v + root.counts["missing"])
