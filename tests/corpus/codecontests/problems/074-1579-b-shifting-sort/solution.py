t=int(input())
for i in range(t):
    n=int(input())
    arr=list(map(int,input().split()))
    s=sorted(arr)
    cnt=0
    lst=[]
    for i in range(n):
        if arr[i]==s[i]:
            continue
        for j in range(i+1,n):
            if arr[j]==s[i]:
                cnt+=1
                lst.append([i+1,j+1,j-i])
                for k in range(j,i,-1):
                    arr[k]=arr[k-1]
                arr[i]=s[i]
                # print(arr)
                break
    print(cnt)
    for i in lst:
        print(*i)
