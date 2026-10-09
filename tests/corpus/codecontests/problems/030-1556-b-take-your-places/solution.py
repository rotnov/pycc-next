import sys
input=sys.stdin.readline
INF=int(1e9)+7
dx=[-1,0,1,0]
dy=[0,1,0,-1]

def solve():
    n=int(input())
    data=list(map(int,input().split()))
    even=0
    odd=0
    for i in data:
        if i%2==0:
            even+=1
        else:
            odd+=1
    if n%2==0:
        if even>n//2 or odd>n//2:
            print(-1)
            return
    else:
        if even>n//2+1 or odd>n//2+1:
            print(-1)
            return
    if n%2==0:
        result1=0
        result2=0
        cnt=0
        for i in range(n):
            if data[i]%2==0:
                result1+=abs(cnt*2-i)
                result2+=abs(cnt*2+1-i)
                cnt+=1
        print(min(result1,result2))
    else:
        if even==n//2+1:
            result1=0
            cnt=0
            for i in range(n):
                if data[i]%2==0:
                    result1+=abs(cnt*2-i)
                    cnt+=1
            print(result1)
        else:
            result1=0
            cnt=0
            for i in range(n):
                if data[i]%2==1:
                    result1+=abs(cnt*2-i)
                    cnt+=1
            print(result1)
            
    
   
t=int(input())
while t:
    t-=1
    solve()
