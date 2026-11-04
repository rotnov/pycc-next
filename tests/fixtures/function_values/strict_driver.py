import strict
print(strict.f(x=3), strict.h(1), strict.h(1, 4), strict.h(1, h=5), strict.s(), strict.s(1, 2, 3))
print(strict.k(2), strict.k(2, k=3), strict.f.__name__, strict.h.__name__)
