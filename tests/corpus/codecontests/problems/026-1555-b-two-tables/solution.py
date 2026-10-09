import sys
import math

testcases=int(input())

while(testcases>0):
    W,H= map(int,sys.stdin.readline().split())
    x1,y1,x2,y2= map(int,sys.stdin.readline().split())
    w,h= map(int,sys.stdin.readline().split())
    a= max(H-y2,y1)
    
    b= max(W-x2,x1)
    
    arr1=[]
    
    if w+(x2-x1)<=W:
        c=max(w-b,0)
        arr1.append(c)
    if h+(y2-y1)<=H:
        d=max(h-a,0)
        arr1.append(d)
    
    if len(arr1)!=0:
        print('%.9f'%min(arr1))

    else:
        print(-1)
        
    testcases-=1
