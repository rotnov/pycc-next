from collections import Counter, defaultdict
import math
import bisect


def getlist():
    return list(map(int, input().split()))

def compute_gcd(x, y):
   while(y):
       x, y = y, x % y
   return x


def maplist():
    return map(int, input().split())

def main():
    t = int(input())
    for num in range(t):
        n = int(input())
        arr = getlist()
        gcd1 = arr[0]
        gcd2 = arr[1]
        for i,numbers in enumerate(arr):
            if i%2==0:
                gcd1 = compute_gcd(gcd1,numbers)
            else:
                gcd2 = compute_gcd(gcd2,numbers)
        # print(gcd1,gcd2)
        i = 1
        flag=  True
        while i<n and gcd1>1:
            if arr[i]%gcd1==0:
                flag = False
                break
            i+=2
        if flag is True and gcd1>1:
            print(gcd1)
        else:
            i = 0
            flag1 = True
            while i<n and gcd2>1:
                if arr[i]%gcd2==0:
                    flag1 = False
                    break
                i+=2
            if flag1 is True and gcd2>1:
                print(gcd2)
            else:
                print(0)


main()
