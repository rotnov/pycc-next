from bisect import bisect_left
n=int(input())
b=list(map(int,input().split()))
pre = [0]



res=[]
for j in range(n):
    res.append(bin(b[j]).count('1'))
    pre.append(pre[-1]+res[-1])


ans = 0
e = 1
o = 0
for j in range(n):
    if pre[j+1] % 2 == 0:
        ans += e
        e += 1
    else:
        ans += o
        o += 1

    s=0
    m=0
    i=j
    while(i>=max(0,j-62)):
        s+=res[i]
        m=max(m,res[i])
        if s%2==0 and s<2*m:
            ans+=-1
        i+=-1
print(ans)
