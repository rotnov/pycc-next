for i in range(int(input())):
    a,b,c=map(int,input().split())
    if(a%2==0 and b%2==0 and c%2==0):
        print(0)
    elif(a%2!=0 and b%2!=0 and c%2!=0):
        print(0)
    elif(b%2==0 and a%2!=0 and c%2!=0):
        print(0)
    elif(a%2==0 and b%2==0 and c%2!=0):
        print(1)
    elif(a%2!=0 and b%2==0 and c%2==0):
        print(1)
    elif(a%2!=0 and b%2!=0 and c%2==0):
        print(1)
    elif(a%2==0 and b%2!=0 and c%2!=0):
        print(1)
    elif(a%2==0 and b%2!=0 and c%2==0):
        print(0)
