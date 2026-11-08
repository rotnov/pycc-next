# `~` on a bool is the int inversion (`~True == -2`, `~False == -1`).
# CPython 3.15 also writes a DeprecationWarning to stderr for it, which pycc
# does not (docs/DEVIATIONS.md); stdout, the exception and the status match.


def invert(flag: bool) -> int:
    return ~flag


def main() -> None:
    t: bool = True
    f: bool = False
    print(~True, ~False, ~t, ~f)
    print(invert(t), invert(f), invert(3 > 2), invert(not t))
    print(~t + 1, ~~f, -(~t), ~f * ~t, [~b for b in [True, False]])


main()
print(~False)
raise ValueError(f"inverted {~True}")
