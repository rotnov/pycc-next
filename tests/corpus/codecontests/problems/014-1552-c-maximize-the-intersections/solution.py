def ch(x,y):
    if (y[0]<x[0]<y[1] and not(y[0]<x[1]<y[1])) or (y[0]<x[1]<y[1] and not(y[0]<x[0]<y[1])):
        return 1
    return 0
    
t = int(input())
for i in range(t):
    n, k = [int(i) for i in input().split()]
    m = []
    use  = [0]*(2*n+1)
    for j in range(k):
        l,r = [int(i) for i in input().split()]
        if l>r:
            l,r = r,l
        use[l]=1
        use[r]=1
        m.append([l,r])
    dontuse = []
    for j in range(1, 2*n+1):
        if not(use[j]):
            dontuse.append(j)
    ll = len(dontuse)
    for j in range(ll//2):
        m.append([dontuse[j],dontuse[j+ll//2]])
    m.sort()
    ans = 0
    for j in range(n):
        for k in range(j+1, n):
            if ch(m[j],m[k]):
                ans+=1
    print(ans)
        
        
        
    
    
