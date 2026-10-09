t=int(input())
while t:
    t-=1
    s=input()
    k,n,m=[int(x) for x in input().split()]
    a=[int(x) for x in input().split()]
    b=[int(x) for x in input().split()]
    i,j=0,0
    ans=[]
    while len(ans)<n+m:
        if i<n and a[i]==0:
            ans.append(0)
            k+=1
            i+=1
        elif j<m and b[j]==0:
            ans.append(0)
            k+=1
            j+=1
        elif i<n and  a[i]<=k:

            ans.append(a[i])
            i+=1
        elif j<m and  b[j]<=k:
            ans.append(b[j])
            j+=1
        else:
            break
    if len(ans)==n+m:
        print(*ans)
    else:
        print(-1)
