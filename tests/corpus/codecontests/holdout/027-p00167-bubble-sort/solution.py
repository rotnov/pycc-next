def bubble_sort(n):
    arr = [int(input()) for _ in range(n)]
    cnt = 0
    for i in range(n):
        for j in range(n-1, i, -1):
            if arr[j] < arr[j-1]:
                arr[j], arr[j-1] = arr[j-1], arr[j]
                cnt += 1
    return cnt

while True:
    n = int(input())
    if n == 0: break
    print(bubble_sort(n))
