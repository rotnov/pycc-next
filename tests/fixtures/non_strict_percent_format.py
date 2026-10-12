# Non-strict mode: `str % args` is CPython's own printf-style formatting,
# and `sys.stdout.write` takes the CPython `str` it produces.
import json
import sys

arr = [3, 1, 2]
print('%.9f' % min(arr))
print("%d items, %s" % (len(arr), "ok"))
print("%5.2f|%-4d|%x|%r|%%" % (2.0 / 3, 7, 255, "q"))
sys.stdout.write('%d ' % (arr[0] + 1))
sys.stdout.write("\n")


class Tree:
    def __init__(self, n):
        self.b = [i * i for i in range(n)]

    def show(self):
        for i in range(len(self.b)):
            sys.stdout.write('%d ' % (self.b[i]))
        print()


Tree(4).show()
s = "%s-%s"
print(s % ("a", 1))
try:
    print("%d" % "x")
except TypeError as e:
    print("TypeError", e)
try:
    print("%d %d" % (1,))
except TypeError as e:
    print("TypeError", e)
try:
    sys.stdout.write(json.loads("5"))
except TypeError as e:
    print("TypeError", e)

# A `str` method given a CPython object argument is CPython's own method.
words = json.loads('["a", "b", "c"]')
print(", ".join(words), "-".join(w.upper() for w in words), "x".count(json.loads('"x"')))
try:
    print(" ".join(json.loads("[1, 2]")))
except TypeError as e:
    print("TypeError", e)
