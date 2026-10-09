from math import sqrt
try:
    while 1:
        a, l, x = map(int, input().split())
        print("%.10f" % (l*sqrt(x*(2*l+x))/2 + a*sqrt(4*l**2 - a**2)/4))
except EOFError:
    ...
