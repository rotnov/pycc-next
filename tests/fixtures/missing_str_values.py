NAMES = {"a": "x", "b": "y"}
COUNTS = {"a": 1}
LETTERS = ["p", "q"]
NUMBERS = [1, 2]


def missing_str(k: str) -> str:
    try:
        return NAMES[k]
    except KeyError:
        return "-"


def missing_int(k: str) -> int:
    try:
        return COUNTS[k]
    except KeyError:
        return -1


def out_of_range(i: int) -> str:
    try:
        return LETTERS[i]
    except IndexError:
        return "-"


def out_of_range_int(i: int) -> int:
    try:
        return NUMBERS[i]
    except IndexError:
        return -1


def pop_empty(n: int) -> str:
    xs = LETTERS[:n]
    try:
        return xs.pop()
    except IndexError:
        return "-"


print(missing_str("a"), missing_str("zz"), missing_int("zz"))
print(out_of_range(5), out_of_range(-1), out_of_range_int(3))
print(pop_empty(0), pop_empty(1))
try:
    print(NAMES["nope"])
except KeyError as e:
    print("KeyError", e)
if __name__ == "__main__":
    print(NAMES["nope"])
