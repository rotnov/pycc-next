n, m = map(int, input().split())
l = sorted(map(int, input().split()))
t, b = l[::-1], -m
for a in l:
    while b < a:
        if a <= b + m:
            n -= 1
        b = t.pop()
print(n)
