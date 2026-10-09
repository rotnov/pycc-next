from collections import deque
n,m,k=list(map(int,input().split()))
arr=list(map(int,input().split()))
d=deque()
for i in arr:
    d.append(i)

chances=curr=tot=0
# print("WORKING")
while tot<m:
    # print("S")
    if (d[0]-curr)%k==0:
        p=(d[0]-curr)//k
    else:p=((d[0]-curr)//k)+1
    temp=curr
    while tot<m and d[0]-temp<=(p*k):
        d.popleft()
        curr+=1
        tot+=1
    chances+=1
print(chances)
