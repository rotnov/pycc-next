t=int(input())
for i in range(t):
    n=int(input())
    arr=list(map(int,input().split()))
    srt=sorted(arr)
    x=0
    while not arr==srt:
        for a in range(0,int((n-1)/2)):
            if arr[2*a]>arr[2*a+1]:
                one=arr[2*a]
                two=arr[2*a+1]
                arr[2*a]=two
                arr[2*a+1]=one
        x+=1
        if arr==srt:
            break
        for b in range(0,int((n-1)/2)):
            if arr[2*b+1]>arr[2*b+2]:
                one=arr[2*b+1]
                two=arr[2*b+2]
                arr[2*b+1]=two
                arr[2*b+2]=one
        x+=1
    print(x)
