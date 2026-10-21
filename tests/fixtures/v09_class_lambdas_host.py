# v0.9: Python code calling `v09_class_lambdas`'s lambda methods
# (`tests/v09_native_state.rs` imports the fixture as `m`, compiled or not).
from m import Table

t = Table(["xy"])
print(Table.key(t, "abc"), t.key("q"), Table.make(("z",)).rows, Table.scaled(t, 1))
try:
    t.key()
except TypeError as e:
    print("TypeError", type(e).__name__)
