for _ in range(int(input())):
    n,k = list(map(int,input().split(" ")))
    ls = list(map(int,input().split(" ")))
    p10s = [10**i for i in ls]
    ans = 0
    used = 0
    kk = k
    last = 1
    for i,j in zip(p10s,p10s[1:]):
        if kk*i == j-1:
            ans += j + (kk-1)*i
            kk = 0
            break
        if kk*i >= j:
            ans += (j-1)-(i-1) - (1 if i==1 else 0)
            used += (j//i)-1 - (1 if i==1 else 0)
            kk = k-used
            last = j
        else:
            ans += kk*i
            kk = 0
            break
    if kk>0:
        ans += kk*last
    print(ans+1)
