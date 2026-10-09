x=input()
x=x.replace("+","")
x=sorted(x)
for i in range(1,2*len(x)-1,2):
    x.insert(i,"+")
x=''.join(x)
print(x)
    
