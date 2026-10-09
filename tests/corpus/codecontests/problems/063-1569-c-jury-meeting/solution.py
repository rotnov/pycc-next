mod = 998244353
import sys
input = sys.stdin.readline

def ncr(n, r):
	return (fact[n]*pow((fact[r]*fact[n-r])%mod, mod-2, mod))%mod

fact = [1, 1]
for i in range(2, 200002):
	fact.append((fact[-1]*i)%mod)


for nt in range(int(input())):
	n = int(input())
	a = list(map(int,input().split()))
	m = max(a)
	if a.count(m)>1:
		print (fact[n])
		continue

	c = a.count(m-1)

	if c==0:
		print (0)
		continue

	ans = fact[n]
	for i in range(c, n):
		ans -= (ncr(i, c) * fact[c] * fact[n-c-1])%mod
		ans = ans%mod
	print (ans)
