"""Base classes another module subclasses."""


class Animal:
    legs: int = 4

    def __init__(self, name: str) -> None:
        self.name = name

    def speak(self) -> str:
        return self.name + " makes a sound"

    def describe(self) -> str:
        return self.speak() + " on " + str(self.legs) + " legs"


def loudest(animals: list[Animal]) -> str:
    best = ""
    for animal in animals:
        if len(animal.speak()) > len(best):
            best = animal.speak()
    return best
