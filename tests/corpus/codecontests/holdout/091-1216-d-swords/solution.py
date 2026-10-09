n= int(input())
s = list(map(int,input().split()))
s.sort()
maxm = s[n-1]
ans = 0
def computeGCD(x, y): 
  
   while(y): 
       x, y = y, x % y 
  
   return x 
a = maxm-s[0] 
for i in range(1,n-1):
    a = computeGCD(a,maxm-s[i])
    
for i in range(0,n-1):
    ans += maxm - s[i]
print(ans//a,a)
    
