n=int(input())
arr=list(map(int,input().split()))
arr.sort()
even=[]
odd=[]
e=0
o=0
for i in arr:
	if (i%2)==0:
		even=even+[i]
		e=e+1
	else:
		odd=odd+[i]
		o=o+1
if (e>o) and (e-o)>1:
	print(sum(even[:(e-o-1)]))
elif (o>e) and (o-e)>1:
	print(sum(odd[:(o-e-1)]))	
else:
	print(0)
	
		
		
	

	 
