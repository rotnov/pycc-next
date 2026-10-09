t = int(input())
for i in range(t):
    mass = input()
    mass = list(mass)
    h = len(mass)
    if h > 2 :
        if mass.count('N') == 1:print("NO")
        else:print("YES")
    else:
        if mass[0]==mass[1]:
            print("YES")
        else:print("NO")
