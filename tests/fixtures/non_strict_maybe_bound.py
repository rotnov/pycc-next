# Non-strict mode: a name bound on some paths only is read with a run-time
# check, raising CPython's NameError (a global) or UnboundLocalError (a
# function local) when no binding ran.
def answer(n, b):
    for i in range(1, n + 1):
        if i not in b:
            root = i
            break
    out = []
    for i in range(1, n + 1):
        if i == root:
            continue
        out.append((root, i))
    return out


print(answer(4, [1, 3]))
try:
    print(answer(2, [1, 2]))
except NameError as e:
    print("caught", e)

ans = []
for t in range(3):
    if t % 2 == 0:
        ans0 = t * 10
    ans.append(str(ans0))
print(" ".join(ans))

for q in [[1, 2], [3]]:
    pass
print(q)

for t in range(2):
    cnt = [0] * 3

    def bump(d):
        cnt[d] += 1

    bump(t)
    print(cnt)


def f(flag):
    if flag:
        v = "set"
    return v


print(f(True))
try:
    f(False)
except NameError as err:
    print("caught", err)
if len(ans) > 5:
    late = 1
print(late)
