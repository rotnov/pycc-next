# l,r = list(map(int,input().split()))


# l<=r

t = int(input())

for i in range(t):
    l,r = list(map(int,input().split()))
    
    if l == r:
        print(0)
    elif int(r/2)+1 >l:
        print(r%(int(r/2)+1))
    else:
        print(r%l)
