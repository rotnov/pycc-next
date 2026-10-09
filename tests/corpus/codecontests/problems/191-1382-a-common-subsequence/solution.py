# list( map(int, input().split()) )
rw = int(input())
for ewqr in range(rw):
    n, m = map(int, input().split())
    a = list(map(int, input().split()))
    b = list(map(int, input().split()))
    aset = set(a)
    bset = set(b)
    c = aset & bset
    c = list(c)
    if c == []:
        print('NO')
    else:
        print('YES')
        print(1, c[0])
        
