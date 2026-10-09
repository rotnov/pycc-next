from sys import*
input= stdin.readline
t=int(input())
for _ in range(t):
    s=input().strip()
    l=[-1]*26
    c=0
    for i in range(len(s)):
        x=ord(s[i])-97
        if(l[x]==-1):
            c+=1
            l[x]=i
        elif(l[x]!=-2):
                c+=1
                l[x]=-2
    print(c//2)
