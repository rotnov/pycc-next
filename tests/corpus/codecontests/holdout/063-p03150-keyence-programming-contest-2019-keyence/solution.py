S = input()
k = "keyence"
n = len(S)-7

for i in range(len(S)-n+1):
    if S[:i]+S[i+n:] == k:
        print("YES")
        break
else:
    print("NO")
