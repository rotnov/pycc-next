"""Subclasses of a class defined in another module."""

import base
from base import Animal


class Dog(base.Animal):
    def speak(self) -> str:
        return self.name + " barks"


class Bird(Animal):
    legs = 2

    def speak(self) -> str:
        return self.name + " sings loudly"


def make_dog(name: str) -> Dog:
    return Dog(name)
