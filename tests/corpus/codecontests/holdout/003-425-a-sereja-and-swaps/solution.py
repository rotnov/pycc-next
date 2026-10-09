#!/usr/local/bin/python3
n, k = map(int, input().split())
a = list(map(int, input().split()))
r_sum = a[0]
for l in range(n):
	for r in range(l, n):
		inside = sorted(a[l:r+1])
		outside = sorted(a[:l] + a[r+1:], reverse=True)
		t_sum = sum(inside)
		for i in range(min(k, len(inside), len(outside))):
			if outside[i] > inside[i]:
				t_sum += (outside[i] - inside[i])
			else:
				break
		if t_sum > r_sum:
			r_sum = t_sum
print(r_sum)
