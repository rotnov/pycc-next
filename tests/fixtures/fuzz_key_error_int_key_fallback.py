# Found by the differential fuzzer (fuzz/): in a `--non-strict` native program
# with code running in CPython (this class: `object` annotations), an
# uncaught `KeyError` for an `int` key printed `KeyError: '6'`.
class Base:
    def __eq__(self, other: object) -> bool:
        return self is other


print(Base() == Base())
{4: "x", 3: "3"}[6]
