from typing import Any, List, MutableMapping, Optional, Set


class BlockState:
    src: str
    tokens: List[Any]
    cursor: int
    parent: Any
    env: MutableMapping[str, Any]
    lazy_line_starts: Set[int]

    def __init__(self, parent: Optional[Any] = None) -> None:
        self.src = ""
        self.tokens = []
        self.cursor = 0
        self.parent = parent
        self.lazy_line_starts = set()
        if parent:
            self.env = parent.env
        else:
            self.env = {"ref_links": {}}

    def child_state(self, src: str) -> "BlockState":
        child = self.__class__(self)
        child.src = src
        return child

    def sibling(self) -> "BlockState":
        return type(self)(self.parent)

    def depth(self) -> int:
        d = 0
        parent = self.parent
        while parent:
            d += 1
            parent = parent.parent
        return d


root = BlockState()
child = root.child_state("text")
grand = child.child_state("more")
print(grand.depth(), grand.src, grand.env is root.env)
print(child.sibling().depth(), isinstance(grand, BlockState))
grand.lazy_line_starts.add(3)
print(sorted(grand.lazy_line_starts), root.tokens, root.cursor)
print(BlockState(grand).depth())
grand.env["missing"]["x"]
