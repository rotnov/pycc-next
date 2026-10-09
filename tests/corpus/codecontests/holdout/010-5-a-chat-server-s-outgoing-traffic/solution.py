import sys
n=0
ans=0
while True:
    i=sys.stdin.readline().strip()
    if len(i)<=1:
        break
    if i[0]=="+":
        n+=1
    elif i[0]=="-":
        n-=1
    else:
        ans+=(len(i.split(':')[1]))*n
print(ans)
