# Non-strict: unannotated functions whose return the checker types from the
# body as a *boxed* tuple (an element other than int/float/bool), next to
# fallback code. The build publishes compiled functions to the fallback
# code, and emits an export thunk for every public function: a boxed tuple
# is one pointer there, not an aggregate to flatten.


def split(text, n):
    head = text[:n]
    return head, len(text) - n


def label(name, xs):
    return name.upper(), [x * 2 for x in xs]


def both(a, b):
    return a + b, b + a


def first(pair):
    return pair[0] * pair[1]


def scalars(n):
    return n, n * 0.5


def pick(items, i):
    return items[i], i


print(split("hello", 2))
a, b = split("abc", 1)
print(a, b)
print(split("", 0))
print(label("x", [1, 2]))
print(label("", [0]))
print(both("ab", "cd"))
print(first(("ab", 3)))
print(first(("", 0)))
print(scalars(3))
total = 0
for word in ["one", "three", "seven"]:
    head, rest = split(word, 2)
    total += len(head) + rest
print(total)
# The fallback: `eval` runs in CPython, so the build embeds it.
print(eval("6 * 7"))
print(pick(["p", "q"], 1))
print(pick(["p", "q"], 5))
