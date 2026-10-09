from collections import Counter
     
t = int(input())
     
for i in range(t):
    s = input()
        
    c = Counter(s)
        
    x = c['A'] + c['C'] == c['B']
    print('YES' if x else 'NO')
