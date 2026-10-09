n=int(input())
S = []
for i in range(n):
    S.append(input()) 
for t in ["AC","WA","TLE","RE"]:
    print(f"{t} x {S.count(t)}")
