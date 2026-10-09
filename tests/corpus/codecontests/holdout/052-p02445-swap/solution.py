n = int(input())
num = list(map(int, input().split()))

q = int(input())
for _ in range(q):
    b, e, t = map(int, input().split())
    for i in range(e-b):
        num[b+i], num[t+i] = num[t+i], num[b+i]

print(' '.join(str(n) for n in num))
