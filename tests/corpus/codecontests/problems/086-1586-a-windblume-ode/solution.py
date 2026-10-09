def prime_list(n):
    sieve = [True] * n

    m = int(n ** 0.5)
    for i in range(2, m + 1):
        if sieve[i] == True:           
            for j in range(i+i, n, i):
                sieve[j] = False

    return [i for i in range(2, n) if sieve[i] == True]

lst = prime_list(20001)


for _ in range(int(input())):
    n = int(input())
    arr = list(map(int,input().split()))


    if sum(arr) not in lst:
        ans = [i for i in range(1,n+1)]
        print(n)
        print(' '.join(map(str,ans)))

    else:
        ans = [i for i in range(1,n+1)]
        for i in range(n):
            if arr[i]%2 == 1:
                ans.pop(i)
                break
        print(n-1)
        print(' '.join(map(str,ans)))
