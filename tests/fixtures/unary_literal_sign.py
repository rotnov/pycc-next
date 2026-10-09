# #602 (Part 1 of #573): a source-level `+`/`-` applied directly to a numeric
# literal folds into that literal's own value.
#
# Covered here: expression position (assignment, arithmetic, comparison,
# function argument, function default-free parameter passing, `print`) and
# `match` value-pattern position, for both `int` and `float`. Mapping-key
# position is covered by `pycc_hir`'s own unit tests instead -- mapping
# patterns have no end-to-end codegen fixture yet, so exercising them here
# would test an unrelated gap.

a = -5
b = +5
print(a)
print(b)
print(a + b)
print(b - a)
print(a * 3)

f = -1.5
g = +1.5
print(f)
print(g)
print(f + g)

print(-7)
print(+7)
print(-2.5)

print(a < 0)
print(b > 0)


def offset(n: int) -> int:
    return n + -10


print(offset(3))
print(offset(-3))


def classify(n: int) -> str:
    match n:
        case -1:
            return "minus one"
        case 0:
            return "zero"
        case -2 | -3:
            return "minus two or three"
        case _:
            return "other"


print(classify(-1))
print(classify(0))
print(classify(-2))
print(classify(-3))
print(classify(9))
