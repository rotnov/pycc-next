I = input
for _ in range(int(I())):
       n = int(I())+1
       s = 0
       for i in range(1,n//2):
              s += 8*i*i
       print(s)
