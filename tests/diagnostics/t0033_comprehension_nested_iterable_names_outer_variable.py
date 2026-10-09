# The inner comprehension iterates the outer loop variable, an `int`, not
# the module-level list `x` (CPython raises `TypeError` here at run time).
x = [5, 6]
print(len([len([y for y in x]) for x in range(3)]))
