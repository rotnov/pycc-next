a = int(input())
for i in range(a):
  ok = dict()
  p1, p2 = map(int, input().split())
  p3 = list(map(int, input().split()))
  op = p3[::]
  for j in range(p1 * p2):
    ok[p3[j]] = []
  for j in range(p1 * p2):
    ok[p3[j]].append(j)
  p3 = sorted(p3)
  miss = []
  for j in range(p1 * p2 - 1, -1, -1):
    miss.append(p3[p1 * p2 - 1 - j])
  lol = []
  for t in miss:
    ok[t] = sorted(ok[t])[::-1]
  for j in range(p1 * p2):
    lol.append(ok[miss[j]][-1])
    ok[miss[j]].pop()
  z = []
  for j in range(p1 * p2):
    z.append(0)
  stack = set()
  rez = 0
  for j in range(p1 * p2):
    if j % p2 == 0:
      stack = set()
    for k in stack:
      if k < lol[j] and op[k] != op[lol[j]]:
        rez += 1
    stack.add(lol[j])
  print(rez)
