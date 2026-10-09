for _ in range(int(input())):
    n = int(input())
    s = list(map(int, input().split()))
    r = 0
    for i in range(n):
        r = max(r, s[i] - i - 1)
    print(r)
