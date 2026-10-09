import sys
import math
import bisect
from math import sqrt
def input():    return sys.stdin.readline().strip()
def iinput():   return int(input())
def rinput():   return map(int, sys.stdin.readline().strip().split()) 
def get_list(): return list(map(int, sys.stdin.readline().strip().split())) 
mod = int(1e9)+7

n, m = rinput()
p = [0] + get_list()
d = {i:set() for i in range(1, n+1)}

for _ in range(m):
	u, v = rinput()
	d[u].add(v)

last = p[n]
target = n

for i in range(n-1, 0, -1):

	for j in range(i, target):
		if p[j+1] in d[p[j]]:
			p[j], p[j+1] = p[j+1], p[j]
		else:
			break	

	if p[target]!=last:
		target -= 1

print(n-target)

# 3 1 4 5 2
