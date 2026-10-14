import collections
from collections import deque


def _bfs(n: int) -> list[int]:
    dist = [-1] * n
    dist[0] = 0
    q = deque([0])
    while q:
        u = q.popleft()
        for v in (u * 2 % n, (u + 1) % n):
            if dist[v] < 0:
                dist[v] = dist[u] + 1
                q.append(v)
    return dist


def main0() -> None:
    d: deque[int] = deque()
    d.append(1)
    d.append(2)
    d.appendleft(0)
    print(d, len(d), d[0], d[-1])
    print(d.pop(), d.popleft(), d)
    d.extend([5, 6, 7])
    d.extendleft([9, 8])
    print(d)
    d.rotate(2)
    print(d)
    d.rotate(-1)
    print(d)
    d.rotate()
    print(d, 5 in d, d.index(5), d.count(7))
    for x in d:
        print(x, end=" ")
    print()
    print(list(d), sorted(d), sum(d))
    b = deque([1, 2, 3], maxlen=3)
    b.append(4)
    print(b)
    b.appendleft(0)
    print(b)
    e = deque("abc")
    print(e, e.copy())
    w = deque()
    w.append("x")
    print(w)
    print(_bfs(10))
    q2 = deque([(0, 0)])
    q2.append((1, 2))
    a, c = q2.popleft()
    print(a, c, q2)
    try:
        ed: deque[int] = deque()
        ed.pop()
    except IndexError as err:
        print("IndexError", err)
    em: deque[str] = deque()
    try:
        em.popleft()
    except IndexError as err:
        print("IndexError", err)
    xs = [1, 2]
    try:
        xs.appendleft(0)
    except AttributeError as err:
        print("AttributeError", err)


main0()



def main1() -> None:
    a = deque([1, 2])
    b = deque([1, 2])
    print(a == b, a != b, a == deque([2, 1]))
    print([a, b])
    c = a + b
    print(c, a * 2)
    e = collections.deque(range(5), 2)
    print(e)
    print("yes" if a else "no")
    q: collections.deque[tuple[int, str]] = collections.deque()
    q.append((1, "a"))
    q.appendleft((0, "z"))
    print(q)
    x = q.pop()
    print(x, q)
    big = deque(range(100000))
    total = 0
    while big:
        total += big.popleft()
        if total % 7 == 0:
            big.append(1)
    print(total)
    z = deque([0], 0)
    z.append(1)
    print(z)
    m = deque([1.5, 2.5])
    m.rotate(1)
    print(m)


main1()



def main2() -> None:
    d = deque([1, 2])
    print(d == [1, 2], [1, 2] == d, f"{d}", str(d))
    try:
        d + [3]
    except TypeError as err:
        print("TypeError", err)
    try:
        print(d[5])
    except IndexError as err:
        print("IndexError", err)
    try:
        d.remove(9)
    except ValueError as err:
        print("ValueError", err)
    try:
        d.index(9)
    except ValueError as err:
        print("ValueError", err)


main2()
