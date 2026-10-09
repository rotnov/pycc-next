import sys
input = sys.stdin.buffer.readline 

def gcd(a, b):
    if a > b:
        a, b = b, a
    if b % a==0:
        return a
    return gcd(b % a, a)

def process(A):
    n = len(A)
    if A[0] % 2==0:
        return 'NO'
    x = 2
    for i in range(1, n):
        g = gcd(x, i+2)
        x = x*(i+2)//g
        if A[i] % x==0:
            return 'NO'
        if x > 10**9:
            break
    return 'YES'
    #say the first even number is at index i
    #it must be divisible by i+2 as well
    #and by everything from 2 to i+2
    #then it cannot be removed, ever,
    #since it can only move left
    #and we can stop if this needed factor
    #is > 10**9
    #since all of our numbers are <= 10**9
    
t = int(input())
for i in range(t):
    n = int(input())
    A = [int(x) for x in input().split()]
    print(process(A))
