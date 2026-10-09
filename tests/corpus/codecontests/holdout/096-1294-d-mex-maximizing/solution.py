q,x=map(int,input().split())
sachin=[]
count1=0
count2=0
arr=[]
out=[]
for i in range(x):
	arr.append(0) 
for i in range(q):
	val=int(input())
	arr[val%x]+=1
	while arr[count2%x]>count1:
		count2+=1
		if count2%x==0:
			count1+=1
	out.append(count2)
print('\n'.join(map(str,out)))
