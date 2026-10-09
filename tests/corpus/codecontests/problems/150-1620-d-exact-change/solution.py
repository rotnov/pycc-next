def mincoin(arr):
    zero = 0
    num3 = max(arr)//3 - 1
    mid = num3*3+3
    lower = [0,0,0]
    upper = [0,0]
    for i in arr:
        if i < mid:
            lower[i%3] = 1
        elif i == mid:
            zero = 1
        else:
            upper[i%3-1] = 1
    lower[0] = zero
    if sum(upper) == 2:
        return num3 + 3
    elif sum(upper) == 0:
        return min(2,  sum(lower)) + num3
    elif upper[0] == 1:
        if num3 < 0:
            return sum(upper)
        elif min(arr) == 1:
            return num3 + 2 + lower[2]
        else:
            return num3 + 2 + (lower[0] and lower[2] == 1)
    else:
        if lower[1] == 1:
            return num3 + 3
        else:
            return num3 + 2

        
r = int(input())
res = []
for i in range(r):
    input()
    arr = list(map(int, input().split()))
    res.append(mincoin(arr))
    
for i in res:
    print(i)
