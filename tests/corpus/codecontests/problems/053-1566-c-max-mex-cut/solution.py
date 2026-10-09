def MEX(a, b):
	a, b = min(a, b)
	if a == 0 and b == 0:
		return 0
	if a == 0 and b == 1:
		return 2
	if a == 1 and b == 1:
		return 0
def solve():
	n = int(input())
	a = input()
	b = input()
	ans = 0
	cur = 0
	mex = None
	while cur < n:
		#print(ans)
		x, y = a[cur], b[cur]
		intX, intY = int(x), int(y)
		if intX + intY == 1:
			ans += 2
			cur += 1
			continue
		if intX + intY == 2:
			cur += 1
			if cur == n:
				break
			while cur < n and a[cur] != '0' and b[cur] != '0':
				cur += 1
			if cur == n:
				break
			ans += 2
			cur += 1
		else:
			if cur + 1 == n:
				ans += 1
				break
			if a[cur + 1] == '1' and b[cur + 1] == '1':
				ans += 2
				cur += 2
			else:
				ans += 1
				cur += 1
	print(ans)

t = int(input())
while t:
	t -= 1
	solve()
