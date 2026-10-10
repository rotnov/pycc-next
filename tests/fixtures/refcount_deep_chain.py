# A million-node linked chain (each node holds its predecessor in a list) is
# released when its head is rebound. The release must not recurse once per
# node, or it would overflow the native stack.


class Node:
    def __init__(self, v: int, rest: list["Node"]) -> None:
        self.v = v
        self.rest = rest


def build(n: int) -> Node:
    empty: list[Node] = []
    node = Node(0, empty)
    for i in range(1, n):
        node = Node(i, [node])
    return node


def depth(node: Node) -> int:
    d = 1
    cur = node
    while len(cur.rest) > 0:
        cur = cur.rest[0]
        d += 1
    return d


def main() -> None:
    head = build(1000000)
    print(head.v, depth(head))
    head = build(3)
    print(head.v, depth(head))
    # A reference cycle is never collected (docs/DEVIATIONS.md), but
    # dropping it must not crash or change output.
    loop = build(2)
    loop.rest[0].rest.append(loop)
    print(loop.rest[0].rest[0].v)
    loop = build(1)
    print(loop.v)


main()
