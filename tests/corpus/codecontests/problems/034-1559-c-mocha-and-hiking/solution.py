t = int(input())
for i in range(t):
	n = int(input())
	x = [int(x) for x in input().split()]
	if(x[-1] == 0):
		for i in range(n):
			print(i+1, end = ' ')
		print(n+1)
	elif(x[0] == 1):
		print(n+1, end = ' ')
		for i in range(n):
			print(i+1, end = ' ')
		print()
		
	else:
		r = -1
		for i in range(n-1):
			if(x[i] == 0 and x[i+1] == 1):
				r  = i
				break
		if(r == -1):
			print(-1)
		else:
			for i in range(1, r+2):
				print(i, end=' ')
			print(n+1, end = ' ')
			for i in range(r+2, n+1):
				print(i, end= ' ')
			print()
