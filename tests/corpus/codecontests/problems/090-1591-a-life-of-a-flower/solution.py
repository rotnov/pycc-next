for _ in range(int(input())):
    n = int(input())
    lst = [*map(int, input().split())]
    tall = 1
    age = 0
    for i in range(n):
        if i != 0 and lst[i - 1] == 1 and lst[i] == 1:
            tall += 5
            age = 0
        elif lst[i] == 1:
            tall += 1
            age = 0
        elif lst[i] == 0:
            age += 1
        if age == 2:
            tall = -1
            break
    print(tall)
