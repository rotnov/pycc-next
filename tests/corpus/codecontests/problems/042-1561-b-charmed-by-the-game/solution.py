def run():
    n = int(input())
    for i in range (n):
        a,b = input().split(' ')
        a = int(a)
        b = int(b)
        min1 = 999999
        max1 = 0
        na = a+b-int((a+b)/2)
        for x in range(max (na-b, 0) ,min(a, na)+1,1 ):


            
            brks = na + a - 2*x
            if brks < 0:
                break
            #print (brks)
            if brks<min1:
                min1 = brks
            if brks > max1:
                max1 = brks
        if (a+b)%2==0:
            L = [i for i in range (min1, max1+2, 2)]
            L.sort()
            print(len(L))
            printp(L)
            
        else:
            min2 = 999999
            max2 = 0
            na = a+b-na
            for x in range(max (na - b, 0), min(a, na) +1,1 ):
                
                brks = na + a - 2*x
                if brks < 0:
                    break
                #print (brks)
                if brks<min2:
                    min2 = brks
                if brks > max2:
                    max2 = brks
            
            L = [i for i in range (min1, max1+2, 2)]
            K = [i for i in range (min2, max2+2, 2)] + L
            K.sort()
            print(len(K))
            printp(K)

def printp(X):
    for i in X:
        print(i, end = ' ')
    print()


run()
    
