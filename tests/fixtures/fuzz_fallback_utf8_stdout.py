# Found by the differential fuzzer (fuzz/): under the C/POSIX locale a
# `--non-strict` native program's embedded CPython wrote `sys.stdout` as
# ASCII, so a statement running in CPython (here `type(w).__name__`, not
# compiled yet) that prints non-ASCII text raised UnicodeEncodeError where
# `python3` (UTF-8 Mode, PEP 540) prints it.


def show(w: str) -> None:
    print(type(w).__name__, w, w.upper())


for w in ["Ωmega", "naïve café", "日本語", "😀"]:
    show(w)
print("compiled", "€100")
