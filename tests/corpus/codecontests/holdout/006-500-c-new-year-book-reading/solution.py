n,m=map(int,input().split())
weight=[int(i) for i in input().split()]
order=[int(i) for i in input().split()]
stack=[]
for i in order:
    if i-1 not in stack:
        stack.append(i-1)
#print(stack)
ans=0
for i in order:
    #i=i-1
    currlift=sum(weight[i] for i in stack[0:stack.index(i-1)])
    ans+=currlift 
    temp=i-1 
    stack.remove(i-1)
    stack.insert(0,temp)
    #print(currlift)
    #print(stack)
print(ans)
    
