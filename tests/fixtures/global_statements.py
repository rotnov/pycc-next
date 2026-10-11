cnt: int = 0
x: int = 5
best = ""
total = 0.0
seen = []


def inc(k: int) -> None:
    global cnt
    cnt += k


def f() -> int:
    x = 7
    return x


def g() -> int:
    return x


def put(s: str) -> None:
    global best, seen
    if len(s) > len(best):
        best = s
    seen = seen + [len(s)]


def acc(v: float) -> float:
    global total
    for total in [total + v, total + 2 * v]:
        pass
    return total


def reads() -> list[int]:
    return [cnt * i for i in range(3)]


def dfs(n: int) -> None:
    global cnt
    if n == 0:
        return
    cnt = cnt + 1
    dfs(n - 1)


def only_reads() -> int:
    global x
    return x * 2


def ordered() -> int:
    global cnt
    cnt += bump_first()
    return cnt


def bump_first() -> int:
    global cnt
    cnt = 1000
    return 1


for i in range(5):
    inc(i)
print(cnt, f(), g(), x, only_reads())
for w in ["a", "abc", "ab", "abcd"]:
    put(w)
print(best, seen)
print(acc(1.5), acc(0.25), total)
print(reads())
dfs(10)
print(cnt)
cnt = 100
inc(1)
print(cnt, reads())
for cnt in range(3):
    inc(10)
    print(cnt)
print(cnt)
print(ordered(), cnt)


def halve() -> int:
    global cnt
    cnt = cnt // (cnt - cnt)
    return cnt


print(halve())
