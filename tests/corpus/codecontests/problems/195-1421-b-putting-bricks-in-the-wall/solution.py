l=[]
for _ in range(int(input())):
    n=int(input())
    a=[]
    for i in range(n):
        a.append(list(input()))
    if a[0][1]==a[1][0]:
        if a[n-1][n-2]==a[n-2][n-1]:
            if a[n-1][n-2]==a[0][1]:
                l.append("2")
                l.append("1 2")
                l.append("2 1")
            else:
                l.append("0")
        else:
            if a[n-1][n-2]!=a[0][1]:
                l.append("1")
                l.append(str(n-1)+" "+str(n))
            else:
                l.append("1")
                l.append(str(n)+" "+str(n-1))
    else:
        if a[n-1][n-2]==a[n-2][n-1]:
            if a[n-1][n-2]!=a[0][1]:
                l.append("1")
                l.append("2 1")
            else:
                l.append("1")
                l.append("1 2")
        else:
            if a[0][1]!=a[n-2][n-1]:
                l.append("2")
                l.append("1 2")
                l.append(str(n-1)+" "+str(n))
            else:
                l.append("2")
                l.append("2 1")
                l.append(str(n - 1)+" "+ str(n))
for i in l:
    print(i)
