# A compiled function reading a module global that other code rebinds while
# the module body runs and after it has run.
X: int = 1
NAME: str = "a"


def get() -> int:
    return X


def name() -> str:
    return NAME


print(get(), name())
globals()["X"] = 2
print(get(), name())
exec("X = 3\nNAME = 'b'")
print(get(), name())
exec("X = 4")
exec("X = 5")
print(get(), get(), name())
