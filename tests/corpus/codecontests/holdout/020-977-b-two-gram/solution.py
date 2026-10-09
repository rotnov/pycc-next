n = int(input())
k = str(input()) 
Max = 0
for i in range(n-1):
    t=k[i]+k[i+1]
    z=0
    for j in range(0,n-1):
        s=k[j]+k[j+1]
        if (s==t):
            z=z+1
    #print(z)
    if (z>Max):
        Max=z
        res=t
print(res)
