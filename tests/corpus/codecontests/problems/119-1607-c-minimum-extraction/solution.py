for _ in range(int(input())):
  n=int(input())
  l=list(map(int,input().split()))
  l.sort()
  s=0
  x=l[0]
  mn=x
  for i in range(n):
    x=l[i]-s
    s+=x
    mn=max(x,mn)
    
  print(mn)
