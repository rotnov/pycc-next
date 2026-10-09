n, M = map(int,input().split())
 

if n==1: 
    print(1)
    exit(0)


 
dp = [1]*(n+1)
accu = [0]*(n+1)

accu[1] = 1
accu[2] = 2

for j in range(4,n+1,2):
    dp[j] += dp[2]


 
for i in range(3,n+1):
    dp[i] += accu[i-1]
    dp[i] = dp[i]%M 


    for j in range(2*i,n+1,i):
        dp[j] += dp[i]
        dp[j] = dp[j] % M



    accu[i] = (accu[i-1] + dp[i])%M



 
#print(dp)
#print(accu) 
 
 
print(accu[n])
