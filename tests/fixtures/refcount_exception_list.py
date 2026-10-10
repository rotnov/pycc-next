# Exception instances are not reference counted: a list of them must hold
# them without touching a count they do not have.



def collect(n: int) -> list[ValueError]:
    errors: list[ValueError] = []
    for i in range(n):
        try:
            raise ValueError("e" + str(i))
        except ValueError as caught:
            errors.append(caught)
    return errors


def main() -> None:
    errors = collect(4)
    errors[0] = errors[3]
    last = errors.pop()
    print(len(errors), last)
    copy = errors[1:]
    print(len(copy), copy[0])
    for e in errors:
        print(e)
    errors = collect(2)
    errors.insert(0, errors[1])
    print(len(errors), errors[0])
    errors = []
    print(len(errors))


main()
kept = collect(3)
for _ in range(40):
    churn = collect(2)
    churn.append(kept[1])
    churn = []
try:
    raise kept[1]
except ValueError as again:
    print("caught", again)
