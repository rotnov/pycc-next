# Reduced from nightly fuzz run 42442802353 (cases 259, 838 and 1888): CPython
# writes a DeprecationWarning for `~` on a bool before `SystemExit`'s message.
def b2_main() -> None:
    ~False


b2_main()
~False
raise SystemExit('bye')
