import pkg
from pkg import area, VERSION, sqrt

print(pkg.area(2.0), area(1.0), pkg.VERSION, VERSION)
print(pkg.math.sqrt(16.0), sqrt(9.0), pkg.core.area(1.0))
s = pkg.Shape(3)
print(s.n)
