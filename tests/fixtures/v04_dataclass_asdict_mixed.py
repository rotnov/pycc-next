from dataclasses import asdict, dataclass


@dataclass
class Item:
    name: str
    qty: int
    price: float


@dataclass(eq=False)
class Order:
    id: int
    items: list[Item]
    tags: list[str]


def main() -> None:
    order = Order(7, [Item("pen", 2, 1.5), Item("ink", 1, 4.25)], ["rush"])
    d = asdict(order)
    print(d)
    print(asdict(Item("cap", 0, 0.0)))
    print(len(d), d["id"])


main()
