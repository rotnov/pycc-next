def _demo() -> None:
    x = {"b": 2, "a": 1}
    x["c"] = 3
    x["a"] = 10  # update-in-place; "a" must NOT move to the end
    print(len(x))
    for k in x:
        print(k)
    print(x["a"])
    print(x["b"])
    print(x["c"])


_demo()
