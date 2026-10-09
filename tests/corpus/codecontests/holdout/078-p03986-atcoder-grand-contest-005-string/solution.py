x=input()
s=0;t=0
for i in range(len(x)):
  if x[i]=='S':s+=1
  elif s==0:t+=1
  else:s-=1
print(s+t)
