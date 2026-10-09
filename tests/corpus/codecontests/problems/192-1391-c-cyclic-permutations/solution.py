n = int(input())
M = 10**9+7
fact = [1]*(n+2)
for i in range(2, n+1):
    fact[i] = (i*fact[i-1])%M
print(((fact[n]-pow(2, n-1, M))+M)%M)
