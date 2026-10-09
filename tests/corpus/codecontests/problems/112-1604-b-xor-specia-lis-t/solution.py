# cook your dish here
for _ in range(int(input())):
    n=int(input())
    a=[int(x) for x in input().split()]
    flag=0 
    if(len(a)%2==0):
        flag=1
    else:
        for i in range(n-1):
            if(a[i]>=a[i+1]):
                flag=1
    if(flag==0):
        print("NO")
    else:
        print("YES")
    
