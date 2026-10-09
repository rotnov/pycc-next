class Animal:
    def __init__(self, name: str) -> None:
        self.name = name

    def speak(self) -> str:
        return "..."

class Dog(Animal):
    def __init__(self, name: str) -> None:
        self.name = name

    def speak(self) -> str:
        return "Woof"

class Cat(Animal):
    def speak(self) -> str:
        return "Meow"

a = Animal("Generic")
print(a.speak())

d = Dog("Rex")
print(d.name)
print(d.speak())

c = Cat("Whiskers")
print(c.name)
print(c.speak())
