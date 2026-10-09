m = int(input())
l = [0 for _ in range(m + 1)]
for _ in range(m - 1):
	a,b = map(int, input().split())
	l[a] += 1
	l[b] += 1
if 2 in l:
	print("NO")
else:
	print("YES")
