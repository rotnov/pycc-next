t = int(input())
tests = []
for i in range(t):
    length = int(input())
    tests.append(input())
    
def solve(s):
    streak1 = 0
    streak2 = 0
    for i in range(len(s)):
        if s[i] == "<":
            streak1 +=1
        else:
            break
    for i in range(len(s)):
        if s[-i-1] == ">":
            streak2 +=1
        else:
            break
    return min(streak1, streak2)

res = list(map(lambda x: str(solve(x)), tests))
print("\n".join(res))
