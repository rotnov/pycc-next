# Iterating a str, substring membership, and container methods on a
# computed receiver.
s = "hello world"
vowels = 0
for c in s:
    if c in "aeiou":
        vowels += 1
print(vowels)
for c in "abc":
    print(c, end="")
print()
print("lo" in s, "xyz" in s, "" in s, "w" not in s)
for i, c in enumerate(s):
    if c == "o":
        print(i)
codes = [ord(c) for c in s if c != " "]
print(codes)
n = 3
g = [[-1] for _ in range(n)]
for i in range(n):
    g[i].append(i * 10)
    g[(i + 1) % n].append(i)
print(g)
last = g[0].pop()
print(last, g)
counts = [0] * 26
for c in "banana":
    counts[ord(c) - ord("a")] += 1
print(counts[:3])
