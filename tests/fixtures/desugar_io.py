# Expression-level desugarings (pycc_ast::desugar): sys streams, module
# lambdas, map/filter, starred displays and print(*xs).
import sys
input = sys.stdin.readline
ints = lambda: list(map(int, input().split()))
dec = lambda x: int(x) - 1
sys.setrecursionlimit(10 ** 6)
n = int(input())
xs = ints()
print(n, xs, sum(xs))
a, b = map(int, input().split())
print(a + b, dec("5"))
ys = list(map(lambda s: int(s) * 2, input().split()))
print(*ys)
print(*ys, sep=",")
print("n:", *ys, end="!\n")
evens = list(filter(lambda v: v % 2 == 0, xs))
print(evens, [*xs], [0, *xs, 9])
words = input().split()
print(*words)
sys.stdout.write(" ".join(map(str, xs)) + "\n")
sys.stdout.flush()
rest = sys.stdin.read().split()
print(rest, len(rest))
# The last line has no newline: the exit flushes it.
for w in rest:
    print(w, end=" ")
