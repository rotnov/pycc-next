t = int(input())
for _ in range(t):
  n, m = map(int, input().split())
  m += 1
  ans = 0
  for x in range(30, -1, -1):
    if (n > m):
        break
    if n >> x == m >> x:
        continue
    if m >> x:
      ans |= 1 << x
      n |= 1 << x

  print(ans)
