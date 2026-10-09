t = int(input())
 
for _ in range(t):
    n = int(input())
    a = list(map(int, input().split()))
    
    rok = True
    rrok = True
    
    if n == 2 and a[0] == 0 and a[1] == 0:
        print("No")
    else:
        if n%2 == 0:
            ar = [0]*n
            
            for i in range(n//2):
                ar[i] = i
                ar[n-i-1] = i
            ar[n//2] = n//2
                
            for i in range(1, n-1):
                if a[i] < ar[i]:
                    rok = False
                    
            ar = ar[::-1]
            
            for i in range(1, n-1):
                if a[i] < ar[i]:
                    rrok = False
 
            print("Yes" if (rok or rrok) else "No")
 
        else:
            for i in range(n):
                if a[i] < min(i, n-i-1):
                    rok = False
                    break
 
            print("Yes" if rok else "No")
