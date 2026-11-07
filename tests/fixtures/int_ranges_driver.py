import os

import int_ranges_api as m

# Non-strict: a `range()` bound beyond 2**61 raises `OverflowError` in a
# compiled loop (docs/DEVIATIONS.md), so that case runs only strict.
NON_STRICT = os.environ.get("INT_RANGES_NON_STRICT") == "1"

cases = [
    ("mix", (17, 40)),
    ("mix", (True, 3)),
    ("mix", (-5, 10)),
    ("mix", (2**70, 4)),
    ("mix", (3, -2)),
    ("mix", (1.5, 3)),
    ("mix", ("a", 3)),
    ("window", (-70, 70)),
    ("window", (False, True)),
    ("window", (None, 3)),
    ("scale", (2**64 + 7,)),
    ("scale", (-(2**80),)),
    ("scale", (True,)),
    ("scale", (300,)),
]
if not NON_STRICT:
    cases.append(("window", (2**65, 2**65 + 3)))
for name, args in cases:
    try:
        print(name, getattr(m, name)(*args))
    except Exception as e:
        print(name, type(e).__name__)
