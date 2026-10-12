# Non-strict mode: a display of None elements (`[None] * n`, `(n, None)`)
# is CPython's own list or tuple, as is one of an element no native list
# holds.
S = [None] * 3
S[0] = 5
print(S, S[1] is None, len(S))


def f(n):
    par = [None] * n
    par[1] = 0
    cur = 1
    steps = 0
    while par[cur] is not None:
        cur = par[cur]
        steps += 1
    return cur, steps


print(f(3))
INF = (5, None)
print(INF, INF[1] is None)
print([None, 1], [[None]] * 2)
last = [None] * 4
for i in range(6):
    if last[i % 4] == None:
        last[i % 4] = i
    elif last[i % 4] != None and None != last[i % 3]:
        last[i % 4] = -i
print(last)
