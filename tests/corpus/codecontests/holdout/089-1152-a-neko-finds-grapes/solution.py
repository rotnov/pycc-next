n, m = map(int, input().split())
a = list(map(int, input().split()))
b = list(map(int, input().split()))
aeven = 0
aodd = 0
beven = 0
bodd = 0
for i in range(n):
    if a[i] % 2 == 0:
        aeven += 1
    else:
        aodd += 1

for i in range(m):
    if b[i] % 2 == 0:
        beven += 1
    else:
        bodd += 1

print(min(aeven, bodd)+min(aodd, beven))
