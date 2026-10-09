l=len
_,*t=open(0)
for p in t:
 x,k=p.split();k=int(k);n=x
 while l(set(x))>k:x=str(int(x)+1).strip('0')
 print(x+(l(n)-l(x))*min(x+'0'*(l(set(x))<k)))
