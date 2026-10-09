n,m,k=map(int,input().split())
p=list(map(int,input().split()))
s=list(map(int,input().split()))
c=set(map(int,input().split()))
d={}
for i in range(n):
    if s[i] not in d:
        d[s[i]]=[-1]
    if p[i]>d[s[i]][0]:
        d[s[i]]=(p[i],i)
st=set()
for i in d:
    st.add(d[i][1]+1)
#print(c,st)
c=c.difference(st)
print(len(c))
