t = int(input())
for tc in range(t):
    n = int(input())
    arr = list(map(int,input().split()))
    newarr = {}
    for i in range(n):
        try:
            newarr[arr[i]*n]+=1
        except:
            newarr[arr[i]*n]=1
    s = sum(arr)
    ans = 0
    for num in arr:
        newarr[num*n]-=1
        x = 2*s-num*n
        try:
            if(newarr[x]):
                ans+=newarr[x]
        except:
            pass
    print(ans)
