for _ in range(int(input())):
  n=int(input())
  s=" "+input()
  if "0" in s:
    i=s.index("0")
    if i<=n//2:
      print(i,n,i+1,n)
    else:
      print(1,i,1,i-1)
  else:
    print(1,n//2,2,n//2+1)
