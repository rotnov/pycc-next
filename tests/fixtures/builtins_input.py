# input(): one stdin line without its newline; EOFError at the end.
n = int(input())
total = 0
for _ in range(n):
    a = input().split()
    total += int(a[0]) * int(a[1])
print(total)
line = input()
print(len(line), line.upper(), "|" + line + "|")
print(input("prompt> "))
try:
    input()
except EOFError as e:
    print("eof:", e)
input()
