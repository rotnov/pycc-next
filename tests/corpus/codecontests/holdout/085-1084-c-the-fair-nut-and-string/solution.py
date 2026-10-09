s=input()
sss=''
for i in s:
    if i in ['a','b']:
        sss+=i 
from itertools import groupby
xxx=[''.join(g) for _, g in groupby(sss)]
xxx=[len(i)+1 for i in xxx if 'a' in i]
ans=1
if len(xxx)==1:
    print((xxx[0]-1)%1000000007)
else:
    for i in xxx:
        ans*=i 
    print((ans-1)%1000000007)
