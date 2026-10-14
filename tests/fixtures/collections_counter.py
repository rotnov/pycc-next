from collections import Counter, defaultdict
import collections


def _word_freq(text: str) -> Counter[str]:
    c: Counter[str] = Counter()
    for w in text.split():
        c[w] += 1
    return c


def main1() -> None:
    c = Counter("abracadabra")
    print(c)
    print(c["a"], c["z"], len(c), "z" in c)
    print(c.most_common(2), c.most_common())
    print(sorted(c.elements()))
    print(c.total())
    c.update("aaz")
    print(c)
    c.subtract(["a", "a"])
    print(c)
    d = Counter([1, 2, 2, 3, 3, 3])
    e = Counter([3, 4])
    print(d + e, d - e, d | e, d & e)
    print(d == Counter([3, 2, 1, 3, 2, 3]), d == e)
    f = _word_freq("the cat the dog the end")
    print(f, f.most_common(1)[0])
    for k, v in sorted(f.items()):
        print(k, v)
    g = Counter()
    g["x"] += 2
    empty: Counter[str] = Counter()
    print(g, empty)
    dd = defaultdict(int)
    for ch in "hello world":
        dd[ch] += 1
    print(dd)
    groups = defaultdict(list)
    for w in ["apple", "avocado", "banana", "cherry", "blueberry"]:
        groups[w[0]].append(w)
    print(groups)
    print(dict(groups))
    sets: defaultdict[int, set[int]] = defaultdict(set)
    sets[1].add(2)
    sets[1].add(3)
    sets[2].add(2)
    for key in sorted(sets):
        print(key, sorted(sets[key]))
    nested = defaultdict(lambda: defaultdict(int))
    nested["a"]["b"] += 1
    nested["a"]["c"] += 2
    print(nested["a"]["b"], nested["a"]["c"], len(nested))
    fives = defaultdict(lambda: 5)
    fives["q"] += 1
    print(fives["q"], fives["r"])
    fl = collections.defaultdict(float)
    fl["x"] += 1.5
    print(fl)
    st = defaultdict(str)
    st[1] += "a"
    st[1] += "b"
    print(st)
    print(dd.get("zz", -1), "zz" in dd)
    try:
        print(dict(dd)["zz"])
    except KeyError as err:
        print("KeyError", err)


main1()


from collections import deque


class Node:
    def __init__(self) -> None:
        self.children: list[int] = []


def _make_node() -> Node:
    return Node()


def _zero_pair() -> list[int]:
    return [0, 0]


def _squares(n: int):
    for i in range(n):
        yield i * i % 5


class Graph:
    adj: defaultdict[int, list[int]]
    deg: Counter[int]

    def __init__(self) -> None:
        self.adj = defaultdict(list)
        self.deg = Counter()

    def add(self, a: int, b: int) -> None:
        self.adj[a].append(b)
        self.adj[b].append(a)
        self.deg[a] += 1
        self.deg[b] += 1


def main2() -> None:
    nodes = defaultdict(_make_node)
    nodes["a"].children.append(1)
    nodes["a"].children.append(2)
    print(len(nodes), nodes["a"].children, len(nodes["b"].children), len(nodes))
    pairs = defaultdict(_zero_pair)
    pairs["x"][1] += 3
    print(pairs["x"], len(pairs))
    print(Counter(_squares(10)))
    base = {"a": 2, "b": 1}
    c = Counter(base)
    c["a"] += 1
    print(c, base)
    g = Graph()
    g.add(1, 2)
    g.add(2, 3)
    print(sorted(g.adj.items()), g.deg.most_common())
    d2 = defaultdict(int, {"k": 1})
    d2["k"] += 1
    d2["j"] += 1
    print(d2)
    cp = d2.copy()
    cp["z"] += 5
    print(cp, len(d2))
    q = defaultdict(deque)
    q[0].append(1)
    q[0].appendleft(0)
    print(q)
    words = "a b a c b a".split()
    print(Counter(words).most_common(1)[0][0])
    cnt = Counter()
    for w in words:
        cnt[w] += 1
    print(cnt == Counter(words), cnt)
    x = Counter("aab")
    x["c"] = 0
    print(x == Counter("aab"), x)
    try:
        none = defaultdict()
        none["x"] = 1
        print(none)
        print(none["y"])
    except KeyError as err:
        print("KeyError", err)


main2()
