# `sys.stdin.buffer` reads bytes, which print and compare unlike str.
import sys
s = sys.stdin.buffer.readline().strip()
print(s)
print(len(s), s == "abc", s[0], list(s[:2]))
