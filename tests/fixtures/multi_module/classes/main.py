import base
import zoo


class Cat(zoo.Animal):
    def speak(self) -> str:
        return self.name + " purrs"


dog = zoo.make_dog("rex")
bird = zoo.Bird("tweety")
cat = Cat("tom")
blob = base.Animal("blob")
print(dog.describe())
print(bird.describe())
print(cat.describe())
print(blob.describe())
print(isinstance(dog, zoo.Dog), isinstance(dog, base.Animal), isinstance(cat, zoo.Animal))
dogs: list[zoo.Dog] = [dog, zoo.Dog("fido")]
print(len(dogs), dogs[1].speak(), base.loudest([blob, base.Animal("ab")]))
print(zoo.Bird.legs, base.Animal.legs, cat.legs, bird.legs)
