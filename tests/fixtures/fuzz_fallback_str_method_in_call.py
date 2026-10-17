# Found by the differential fuzzer (fuzz/): in `--non-strict` mode a `str`
# method pycc does not compile, as the argument of another method call
# (`xs.append(s.removeprefix(p))`), was rewritten as a call through
# `builtins.str` of the *outer* method: `str.append(xs, ...)`.
xs: list[str] = list("")
xs.append("h".removeprefix("t"))
xs.append("abc".removesuffix("c"))
n: int = 7
xs.append(str(n.bit_length()))
print(xs)
