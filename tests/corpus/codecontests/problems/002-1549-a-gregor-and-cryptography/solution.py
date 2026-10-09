number = int(input())
for i in range(number):
    chislo = int(input())
    if chislo%2 == 1:
        print(2, chislo - 1)
    else:
        if chislo %3 == 1:
            print(3, chislo-1)
        elif chislo%3 == 2:
            print(3, chislo - 2)
