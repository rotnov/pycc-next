import sys
input = sys.stdin.readline
n, m = map(int, input().split())

class UnionFind():
  def __init__(self, n):
    self.n = n
    self.root = [-1] * (n + 1)
    self.rnk = [0] * (n + 1)
  def Find_Root(self, x):
    if self.root[x] < 0:
      return x
    else:
      self.root[x] = self.Find_Root(self.root[x])
      return self.root[x]
  def Unite(self, x, y):
    x = self.Find_Root(x)
    y = self.Find_Root(y)
    if x == y:
      return 
    elif self.rnk[x] > self.rnk[y]:
      self.root[x] += self.root[y]
      self.root[y] = x
    else:
      self.root[y] += self.root[x]
      self.root[x] = y
      if self.rnk[x] == self.rnk[y]:
        self.rnk[y] += 1
  def SameQuery(self, x, y): return self.Find_Root(x) == self.Find_Root(y)
  def Count(self, x): return -self.root[self.Find_Root(x)]

uf = UnionFind(n)

e = [[] for _ in range(n + 1)]
edges = []
for i in range(m):
  u, v = map(int, input().split())
  if uf.SameQuery(u, v):
    edges.append((0, 0))
    continue
  uf.Unite(u, v)
  e[u].append((v, i))
  e[v].append((u, i))
  edges.append((u, v))

q = int(input())
qs = [tuple(map(int, input().split())) for _ in range(q)]

table = [0] * m
res = []
for u, v in qs:
  s = [u]
  vis = [0] * (n + 1)
  rev = [0] * (n + 1)
  vis[u] = 1
  while len(s):
    x = s.pop()
    for y, _ in e[x]:
      if vis[y]: continue
      vis[y] = 1
      s.append(y)
      rev[y] = x

  x = v
  res.append([])
  while x != u:
    res[-1].append(x)
    for y, i in e[x]:
      if y == rev[x]:
        table[i] ^= 1
        x = y
        break
  res[-1].append(u)
  res[-1].reverse()


if sum(table) == 0:
  print("YES")
  for r in res:
    print(len(r))
    print(*r)
else:
  print("NO")
  c = [0] * (n + 1)
  for i in range(m):
    if table[i] == 0: continue
    u, v = edges[i]
    c[u] ^= 1
    c[v] ^= 1
  print(sum(c) // 2)
