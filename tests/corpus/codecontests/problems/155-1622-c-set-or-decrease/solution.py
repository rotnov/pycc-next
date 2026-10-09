I=lambda:[*map(int,input().split())]
for _ in[0]*I()[0]:
	n,k=I();a=sorted(I());p=[0];b=1<<50
	for i in range(1,n):p.append(p[-1]+a[i])
	for i in range(n):b=min(max(0,a[0]-(k-p[-1-i])//(i+1))+i,b)
	print(b)
