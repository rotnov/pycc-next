P = print
rg = range
L = len


class Node(object):
    def __init__(self, v: int) -> None:
        self.v = v


n = int(input())
for i in rg(n):
    P("YES") if i % 2 == 0 else P("NO")
    print(i, end=" ") if i > 0 else print("first")
nodes = [Node(k) for k in rg(3)]
P(L(nodes), nodes[-1].v)
P(*[x * x for x in rg(n)], sep=",")
