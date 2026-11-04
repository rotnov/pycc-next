import fvals
print(fvals.LOOKUP["max_digits"] is fvals.max_digits, fvals.PAIR[1] is fvals.places)
print(fvals.chosen("places") is fvals.places, fvals.chosen("max_digits").__name__)
print(fvals.LOOKUP["max_digits"](1, limit=2), fvals.PAIR[1](1, places=2))
print(fvals.bound("places", places=4, limit=10)(5), fvals.bound("max_digits", limit=3)(7))
print(fvals.LOOKUP["places"](1, 2), sorted(fvals.LOOKUP))
try:
    fvals.LOOKUP["max_digits"](1, nope=2)
except TypeError as e:
    print("TypeError", e)
print(fvals.own_name(1, 2), fvals.own_default(1, 2), fvals.own_default(1))
print(fvals.own_default(1, own_default=3), fvals.own_default.__name__)
print(fvals.collect(b=2, a=1), fvals.star(1, 2), fvals.star())
