while True:
    N = int(input())
    if N == 0:
        break
    a = input()
    b = a.split()
  
    x = 0
    for i in range(N//2):
        if b[2 * i] == 'lu' and b[(2 * i)+1] == 'ru':
            x += 1
        if b[2 * i] == 'ru' and b[(2 * i)+1] == 'lu':
            x += 1
        if b[2 * i] == 'ld' and b[(2 * i)+1] == 'rd':
            x += 1
        if b[2 * i] == 'rd' and b[(2 * i)+1] == 'ld':
            x += 1
    print(x)
