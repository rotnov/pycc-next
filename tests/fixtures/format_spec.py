# Format specs (`f"{x:spec}"`, `format(x, spec)`) and printf-style `%`
# formatting of int, bool, float and str, compared with CPython.


def show_int_specs(n: int) -> None:
    for spec in ["", "d", "5d", "<6", "^7", "*>8", "=+8", "+", " ", "-", ",", "_", "012,",
                 "0=12,", "0>12,", "x", "X", "#x", "#X", "o", "#o", "b", "#010b", "_b", "_x",
                 "08", "e", ".2e", "f", ".3f", "g", ".3g", "%", ".1%", "n", "+,"]:
        print(repr(spec), format(n, spec))


def show_float_specs(x: float) -> None:
    for spec in ["", ".2f", "10.3f", "<10.1f", "^11.2e", "+.3e", "E", ".0f", "#.0f", ".0e",
                 "#.0e", "g", ".3g", "#g", "G", ".10g", "%", ".2%", ",", ",.2f", "_.3f",
                 "015,.2f", "0=15.3f", "z.1f", ".3", ".1", "n", "010", "-^9.1f"]:
        print(repr(spec), format(x, spec))


def show_str_specs(s: str) -> None:
    for spec in ["", "s", "5", ">5", "^6", "*^7", ".2", "8.3", "05", "x<4"]:
        print(repr(spec), "[" + format(s, spec) + "]")


def show_printf() -> None:
    x = 3.14159
    n = -42
    s = "hi"
    print('%.9f' % x)
    print('%5.2f|%-5d|%05d|%+d|%x|%#o|%e|%g|%s|%r|%c|%%|%i|%.3s|% d' % (
        x, 42, n, 5, 255, 8, 1234.5, 0.0001, s, s, 65, 7, 'abcdef', 3))
    print('%d' % 3.99, '%s %s' % (True, 2.0), '%5.1f' % 3, '%08.3d|%#05x|%-#8o|' % (n, 255, 8))
    print('[%s]' % s, '%c%c' % ('a', 98), '%r' % 2.5, '%.2e' % 12345, '%X' % 48879)
    big = 10 ** 20
    print('%d|%x|%e|%s' % (big, big, big, big), format(big, ','), f'{-big:_x}')
    for v in [0, 1, 255, -7]:
        print('%3d:%-4x:%o:%5.1e' % (v, v, v, v))
    print('%f %F %g %G' % (float('inf'), float('-inf'), float('nan'), 1e-10))
    print('%05f|%-6f|%+f' % (float('inf'), float('inf'), float('inf')))
    print('%s and %s' % ('a', 'b'), 'no conversions %%')


def show_errors() -> None:
    n = 7
    s = "abc"
    x = 1.5
    for spec in [".2", "c+", "z", "q", ",x", ",,", "_,", "5dd"]:
        try:
            print(format(n, spec))
        except ValueError as e:
            print("ValueError", e)
    for spec in ["d", "+", " ", "#", "=5", ","]:
        try:
            print(format(s, spec))
        except ValueError as e:
            print("ValueError", e)
    for spec in ["d", "x", "c", "."]:
        try:
            print(format(x, spec))
        except ValueError as e:
            print("ValueError", e)
    try:
        print('%c' % 1114112)
    except OverflowError as e:
        print("OverflowError", e)
    try:
        print(format(-1, 'c'))
    except OverflowError as e:
        print("OverflowError", e)


show_int_specs(1234567)
show_int_specs(-255)
show_int_specs(0)
show_float_specs(1234.5678)
show_float_specs(-0.000012345)
show_float_specs(0.0)
show_float_specs(-0.0)
show_float_specs(1e16)
show_float_specs(float("inf"))
show_float_specs(float("nan"))
show_str_specs("abcd")
show_str_specs("")
show_printf()
show_errors()
b = True
print(format(b), format(b, "d"), format(b, ">5"), f"{b}", f"{b:^7}", f"{not b:x}")
w = 2.5
print(f"{w:.9f}", f"{w!r:>8}", f"{'q'!r:^9}", f"{12345678:_}", f"{255:#X}", f"{0.5:.0%}")
total = 0
for i in range(20000):
    total += len(f"{i:08.3f}") + len('%5d' % i) + len(format(i, ","))
print(total)
