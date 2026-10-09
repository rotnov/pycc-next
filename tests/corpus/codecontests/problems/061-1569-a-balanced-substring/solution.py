for _ in range(int(input())):
	_=int(input())
	s = input()
	a=[0]
	b=[0]
	c=0
	for i in s:
		if i=='a':
			a.append(a[-1]+1)
			b.append(b[-1])
		else:
			a.append(a[-1])
			b.append(b[-1]+1)
	for i in range(1,len(a)-1):
		for j in range(i+1,len(a)):
			af = a[j]-a[i-1]
			bs = b[j]-b[i-1]
			if af==bs:
				c=1
				print(i,j)
				break
		if c==1:
			break

	if c==0:
		print(-1,-1)
