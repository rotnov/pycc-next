def lookup(d: dict[str, int], key: str) -> int:
    return d[key]


def raise_key(message: str) -> int:
    raise KeyError(message)


def show(message: str) -> None:
    try:
        raise_key(message)
    except KeyError as e:
        print(e)
        print(f"[{e}]")
        print("<" + f"{e}" + ">")


show("k")
show("")
show("it's")
show("a'b\"c")
show("tab\there")

try:
    raise_key("wide")
except Exception as e:
    print("generic:", e)

try:
    raise ValueError("plain")
except ValueError as e:
    print(e)

table: dict[str, int] = {"a": 1}
print(lookup(table, "a"))
try:
    print(lookup(table, "zz"))
except KeyError as e:
    print("missing", e)

raise KeyError("boom")
