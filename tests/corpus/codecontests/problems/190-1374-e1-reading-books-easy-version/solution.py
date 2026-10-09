n,k,*f = map(int, open(0).read().split())
books = [f[i*3:i*3+3] for i in range(n)]
a = []
b = []
ab = []
ans = float('inf')
for x in books:
    if x[1] == 1:
        if x[2] == 1:
            ab.append(x[0])
        else:
            a.append(x[0])
    elif x[2] == 1:
        b.append(x[0])
m = min(len(a),len(b))
if m + len(ab) >= k:
    a.sort()
    b.sort()
    ab.sort()
    csa = [0]
    for i in range(len(a)):
        csa.append(csa[i]+a[i])
    csb = [0]
    for i in range(len(b)):
        csb.append(csb[i]+b[i])
    csab = [0]
    for i in range(len(ab)):
        csab.append(csab[i]+ab[i])
    for i in range(max(0,k-m),min(k,len(ab))+1):
        ans = min(ans,csab[i]+csa[k-i]+csb[k-i])
    print(ans)
else:
    print(-1)
