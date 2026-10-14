# `from sys import exit`: a string argument is printed to stderr, status 1.
from sys import stdin, exit

line = stdin.readline().strip()
print(line.upper())
if line:
    exit("stopped: " + line)
print("not reached")
