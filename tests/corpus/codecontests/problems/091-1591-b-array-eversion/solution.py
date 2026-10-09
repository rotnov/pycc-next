for t in range(int(input())):
  n=int(input())
  arr=list(map(int,input().split()))
  arr=arr[::-1]
  maxi=arr[0]
  count=0
  for i in range(1,len(arr)):
    if arr[i]>maxi:
      count+=1
      maxi=arr[i]
  print(count)
