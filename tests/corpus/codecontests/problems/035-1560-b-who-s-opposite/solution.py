import math

t = int(input())
for _ in range(t):
    a,b,c = map(int,input().split())
    n = abs(b-a)*2

    diff = abs(b-a)
    if a>n or b>n or c>n:
        print(-1)
        continue
    x1,x2 = c + diff , c-diff
    ans = -1
    if 1<=x1<=n:
        ans = c+diff
    if 1<=x2<=n:
        ans = c-diff
    print(ans)
