# `open(0)` (the builtin `open`) is standard input as a text file.
n, k, *f = map(int, open(0).read().split())
print(n, k, f, sum(f[:n]))
