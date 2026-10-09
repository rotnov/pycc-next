tanku = [5, 7, 5, 7, 7]
while True:
  n = int(input())
  if n==0:
    break
  w = [len(input()) for i in range(n)]
  ans = 0
  for i in range(n):
    sum = 0
    k = 0
    for j in range(i, n):
      sum += w[j]
      if sum == tanku[k]:
        sum = 0
        k += 1
        if k==5:
          ans = i+1
          break
      elif sum > tanku[k]:
        break
    if ans:
      break
  print(ans)
