t = int(input())
for _ in range(t):
    ans = 0
    n, pos, control = map(int, input().split())
    control = min(control, pos - 1)
    not_control = pos - control - 1
    num = n - control - not_control
    a = list(map(int, input().split()))
    for i in range(control + 1):
        tmp = 10 ** 10 + 1
        for j in range(not_control + 1):
            try:
                tmp = min(tmp, max(a[i + j], a[i + j + num - 1]))
            except ReferenceError:
                pass
        ans = max(ans, tmp)
    if ans == 10 ** 10:
        ans = max(a[0], a[-1])
    print(ans)
