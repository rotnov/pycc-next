N = int(input())
p = list(map(int, input().split()))
p.append(0)
cnt = 0
for i in range(N):
  if p[i] == i + 1:
    p[i], p[i+1] = p[i+1], p[i] 
    cnt += 1
print (cnt)
