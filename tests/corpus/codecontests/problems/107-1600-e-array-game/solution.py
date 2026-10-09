length = int(input())
arr = input().split(" ")
arr = list(map(int, arr))


leftMax = 1
rightMax = 1
for i in range(length-1):
    if arr[i]<arr[i+1]:
        leftMax += 1
    else:
        break
for i in range(length-1):
    if arr[-(i+1)]<arr[-(i+2)]:
        rightMax += 1
    else:
        break

if leftMax%2 != 0 or rightMax%2 != 0 or length == 1:
    print("Alice")
else:
    print("Bob")
