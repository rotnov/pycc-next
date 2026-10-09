t = int(input())
for _ in range(1, t + 1):
    a, b = map(int, input().split())
    n = a // 2 + 1
    print(b // n)
