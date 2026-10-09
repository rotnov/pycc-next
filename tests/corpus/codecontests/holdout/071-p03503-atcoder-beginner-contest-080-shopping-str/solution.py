n = int(input())
f = [[int(x) for x in input().split()] for i in range(n)]
p = [[int(x) for x in input().split()] for i in range(n)]
t={i:0 for i in range(1,2**10)}
for i in range(1,2**10):
  d, b={j:0 for j in range(n)}, format(i, "010b")
  for j in range(n):
    for k in range(10):
      if (i>>k)%2&f[j][k]==1: d[j]+=1
    t[i]+=p[j][d[j]]
print(max(t.values()))
