# v0.9: Python code handing `v09_dispatch`'s subclass instances to compiled
# code typed with their base (`tests/v09_native_state.rs` imports the
# fixture as `m`, compiled or not).
from m import Odd, Polygon, Shape, Square, describe, equal, label_of, logged, name_of, via_object

shapes = [Shape(1), Polygon(5), Square(9), Odd(2)]
print([describe(s) for s in shapes])
print([name_of(s) for s in shapes])
print([Shape.describe(s) for s in shapes])
print([Shape.name(s) for s in shapes], Polygon.sides(Square(1)))
print(logged(Square(2)), logged(Polygon(3)))
print([type(s).__name__ for s in shapes], isinstance(Square(1), Polygon))
print([via_object(s) for s in shapes], via_object(3))
print([label_of(s) for s in shapes], [equal(s, Shape(0)) for s in shapes])
