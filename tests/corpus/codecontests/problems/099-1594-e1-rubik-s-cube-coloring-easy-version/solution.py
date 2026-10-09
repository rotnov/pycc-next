n = int(input())
vid = 6
k = 2
m = int(1e9 + 7)
for i in range(n - 1):
    vid = (vid * pow(4, k, m)) % m
    k *= 2
print(vid)
