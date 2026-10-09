#C
N = int(input())
A,B = map(int,input().split())
C = int(input())
T = [int(input()) for i in range(N)]
T.sort(reverse=True)

cal = C
cost = A
for t in T:
    if cal/cost < (cal+t)/(cost+B):
        cal+=t
        cost+=B
    else:
        break
        
print(cal//cost)
