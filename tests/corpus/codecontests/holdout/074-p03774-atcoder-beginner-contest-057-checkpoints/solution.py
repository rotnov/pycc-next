n,m=map(int,input().split())
p=[list(map(int, input().split())) for _ in range(n)]
c=[list(map(int, input().split())) for _ in range(m)]
for i in p:
    d=[abs(i[0]-c[j][0])+abs(i[1]-c[j][1]) for j in range(len(c))]
    print(d.index(min(d))+1)
