import math

nm = input().split()
n = int(nm[0])
m = int(nm[1])

lis = [ 0 for i in range(m+1)]
for _ in range(n) :
    inp =  list(map(int, input().split()))

    inp.pop(0)
    for i in inp:
        lis[i]=1
        prev = i
if sum(lis)==m:
    print("YES")
else:
    print("NO")
