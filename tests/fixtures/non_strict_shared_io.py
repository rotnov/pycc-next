# Native and fallback code read and write the same stdin/stdout: compiled
# `input()`/`print` and CPython-run fallback functions interleave in one
# program, so a line is never read twice or lost and output keeps program
# order. Oracle: CPython runs this file unchanged with the `.stdin` file.
import sys

calls = 0

def ask(prefix):
    # Falls back (a class nested in a function); reads a line through
    # CPython's `sys.stdin`.
    class Tag:
        pass
    global calls
    calls += 1
    tidy = lambda s: s.strip().upper()
    line = input()
    print(prefix, tidy(line))
    return len(line)


def echo_rest():
    # Falls back (a class nested in a function); reads the rest of stdin.
    class Tag:
        pass
    global calls
    calls += 1
    show = lambda s: print("rest:", s.split())
    show(sys.stdin.read())


n = int(input())
print("native first", n)
k = ask("fallback")
print("native again", k, end="!\n")
line = sys.stdin.readline()
print("native line", line.strip())
sys.stdout.write("native write\n")
print(ask("fallback 2"))
w = input()
print("native last", w)
echo_rest()
print("done", calls)
