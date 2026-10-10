# Non-strict: builtins used as values, `exit`, and str/int methods pycc does
# not compile for their arguments go through CPython from compiled code.
from collections import defaultdict

d = defaultdict(int)
words = "b a b".split()
for w in words:
    d[w] += 1
l = len
print(l(words), sorted(d.items()))
ans = [str(v) for v in d.values()]
out = (lambda: ans)()
print("\n".join(out))
print("{} and {}".format(out[0], d["a"]))
M = 2 * len(words)
print((M - 1).bit_length())


class P:
    def __init__(self, v: int) -> None:
        self.v = v


# Instances reach fallback code (the lambda) without the class name being
# read there: the class runs in CPython too.
ps = [P(1), P(2)]
print((lambda: [p.v for p in ps])())
if M > 100:
    exit(0)
print("end")
exit(3)
