q = int(input())
for i in range(q):
    n, m, k = map(int, input().split())
    if m >k or n > k:
        print(-1)
    else:
        print(k - (k-n)%2 - (k-m)%2)
