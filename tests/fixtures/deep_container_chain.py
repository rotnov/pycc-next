# A long chain of instances linked through dicts is freed without recursing
# once per link (CPython's trashcan; a recursive free overflows the stack).


class Node:
    def __init__(self) -> None:
        self.kids: dict[str, Node] = {}


class Pair:
    def __init__(self) -> None:
        self.rest: list[dict[str, Pair]] = []


def dict_chain(n: int) -> Node:
    head = Node()
    cur = head
    for _ in range(n):
        nxt = Node()
        cur.kids["k"] = nxt
        cur = nxt
    return head


def mixed_chain(n: int) -> Pair:
    head = Pair()
    cur = head
    for _ in range(n):
        nxt = Pair()
        cur.rest.append({"k": nxt})
        cur = nxt
    return head


def main() -> None:
    h = dict_chain(400000)
    print(len(h.kids))
    h = Node()
    print("dict chain freed")
    p = mixed_chain(400000)
    print(len(p.rest))
    p = Pair()
    print("mixed chain freed")


main()
