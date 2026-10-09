from enum import Enum, StrEnum, auto

class Color(Enum):
    RED = 1
    GREEN = 2
    BLUE = 3

print(Color.RED.value)
print(Color.GREEN.value)
print(Color.BLUE.value)
print(Color.RED.name)
print(Color.GREEN.name)
print(Color.BLUE.name)

for c in Color:
    print(c.value)
    print(c.name)

class Suit(Enum):
    HEARTS = "hearts"
    SPADES = "spades"

print(Suit.HEARTS.value)
print(Suit.SPADES.name)

for s in Suit:
    print(s.value)
    print(s.name)

class Kind(StrEnum):
    AXIAL = "axial"
    RADIAL = auto()

print(Kind.AXIAL.value)
print(Kind.RADIAL.value)
print(Kind.RADIAL.name)

class Level(Enum):
    LOW = auto()
    MID = auto()
    HIGH = 9

print(Level.LOW.value)
print(Level.MID.value)
print(Level.HIGH.value)
