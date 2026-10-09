ql=[]
for tt in range(int(input())):
	ql.append(input())
l=[]
d=[-1]*1000000
for tt in range(1,len(ql)+1):
	q=list(map(int,ql[-tt].split()))
	if(len(q)==2):
		if(d[q[1]]==-1):
			l.append(q[1])
		else:
			l.append(d[q[1]])
	else:
		if(d[q[2]]==-1):
			d[q[1]]=q[2]
		else:
			d[q[1]]=d[q[2]]

for i in range(1,len(l)+1):
	print(l[-i],end=" ")
