for _ in range(int(input())):
    n=int(input())
    a=list(map(int, input().split()))
    a.sort()
    y=a[0]
    for i in range(1,n//2 + 1):
        print(a[i] , y)
