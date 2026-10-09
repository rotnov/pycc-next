class Boxed:
    def __init__(self) -> None:
        self.tag = 0

    @staticmethod
    def __class_getitem__(key: int) -> int:
        return key * 2


class Named:
    def __init__(self) -> None:
        self.tag = 1

    @classmethod
    def __class_getitem__(cls, key: int) -> int:
        return key + 1000


class Base:
    def __init__(self) -> None:
        self.tag = 2

    @staticmethod
    def __class_getitem__(key: int) -> int:
        return key + 100


class Derived(Base):
    def __init__(self) -> None:
        self.tag = 3


def index_via_hook(key: int) -> int:
    return Boxed[key]


print(Boxed[3])
print(Boxed[0])
print(Named[7])
print(Derived[5])
print(index_via_hook(21))

boxed = Boxed()
print(boxed.tag)


# Issue #693 (PEP 560): annotation-position `ClassName[type_arg]` routes
# through the hook to compute the annotated type. CPython never evaluates an
# annotation at runtime, so `x: Boxed[3] = 6` just binds `x` to `6` there --
# but pycc's static type system resolves the annotation eagerly, and prior
# to #693 it always used `Ty::Instance(Boxed)` regardless of the hook,
# which would have made `x + 1` below a compile-time type error (`Boxed`
# supports no `+` operator). Routing the annotation through
# `__class_getitem__`'s declared `-> int` return type instead makes `x` an
# `int`, so `x + 1` compiles and its runtime value matches CPython's own
# (annotations are inert to CPython, so this only ever exercises ordinary
# `int` arithmetic there).
def annotated_via_own_hook() -> int:
    x: Boxed[3] = 6
    return x + 1


print(annotated_via_own_hook())

# Same routing at module scope, and through a hook inherited via the MRO
# (`Derived` defines no `__class_getitem__` of its own -- `Base` does).
y: Named[1] = 41
print(y + 1)

z: Derived[9] = 8
print(z * 2)
