n = int(input())
size = 200100
total = [0 for _ in range(size)]
for _ in range(n):
  s = sum(map(int, input().split()))
  total[s] += 1

for i in range(size - 1):
  if total[i] % 2:
    print(i, 0)
  total[i + 1] += total[i] // 2
