from sys import stdin
input = stdin.readline

def answer():

    for i in range(1 , n + 1):

        if(i not in b):
            root = i
            break

    for i in range(1 , n + 1):
        if(i == root):continue
        print(root , i)
    
   
    
for T in range(int(input())):

    n , m = map(int,input().split())


    b = set()
    for i in range(m):
        u , v , w = map(int,input().split())

        b.add(v)

    
    answer()
