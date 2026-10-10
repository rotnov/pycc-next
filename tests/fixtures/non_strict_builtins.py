# Non-strict: a builtin call pycc does not compile for its arguments runs
# through CPython's `builtins` module, inside otherwise compiled code.
def best(xs):
    ans = 10 ** 9
    for x in xs:
        ans = min(ans, x * 2)
    return ans


print(best([5, 3, 8]))
print(chr(65 + 2), ord("a"))
seen = set()
seen.add(3)
print(len(seen), 3 in seen)
print(divmod(17, 5), pow(3, 4, 7), bin(10), hex(255))
print(max(3, 4.5), min("b", "a"))
print(sum([1, 2, 3]))
print(max([], default=-1))
print(min(1, "a"))
