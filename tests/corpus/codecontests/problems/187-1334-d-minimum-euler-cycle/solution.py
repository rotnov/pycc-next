from sys import stdin
input = stdin.readline
q = int(input())
for _ in range(q):
	n,l,r = map(int,input().split())
	tab = [-1,1]
	cyk = 0
	for i in range(n):
		cyk += 2*(n-i-1)
		tab.append(cyk + 1)
	tab.append(1012809128301279797787789798789798798)
	i = 0
	start = i
	while True:
		if tab[i + 1] > l:
			start = i
			break
		else:
			i += 1
	#zaczynamy od i X i (x+1) ...
	ind = l
	dlug = 0
	koniec = 0
	while True:
		if koniec:
			break
		target = tab[start+1]
		while ind < target:
			if koniec:
				break
			if ind == (n*(n-1) + 1):
				print(1, end = " ")
				dlug += 1
			else:
				dlug += 1
				if ind%2==1:
					print(i, end = " ")
				else:
					print(n-((target-ind)-1)//2, end = " ")
				ind += 1
			if dlug >= (r-l+1):
				koniec = 1
		start += 1
		i += 1
	print()
