# `X = sys.stdin` / `X = sys.stdout`: X is the stream itself.
import sys

inp = sys.stdin
out = sys.stdout
n = int(inp.readline())
words = inp.readline().split()
for w in words[:n]:
    out.write(w[::-1] + "\n")
out.flush()
