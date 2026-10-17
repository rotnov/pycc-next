"""The pydantic timing workload of `bench/pydantic_check.py --time` (v0.7 accept 4).

Usage: python3 -I bench/pydantic_workload.py TREE REPEAT [PHASES]
  TREE    the directory holding the `pydantic` package to time (compiled or not).
  REPEAT  how many times each phase runs; the time printed is the mean per run.
  PHASES  comma-separated phases after `define` (default `schema,roundtrip`).

Prints one JSON object, seconds per phase:
  import     `import pydantic` and the names used here, once (stdlib imported before).
  define     class creation of a representative model set: nested models, field and
             model validators, a computed field, a field serializer, Field constraints,
             a discriminated union, a self-referencing model, a subclass, a generic
             model parametrized, a pydantic dataclass and a TypeAdapter.
  schema     `model_json_schema()` of four of them, in validation and serialization
             mode.
  roundtrip  validate 200 nested dicts, dump them to Python and to JSON, validate the
             JSON back; mostly pydantic-core's own time.
"""
import datetime
import decimal
import enum
import json
import sys
import time
import typing
import uuid

sys.path.insert(0, sys.argv[1])
REPEAT = int(sys.argv[2])
PHASES = sys.argv[3].split(",") if len(sys.argv) > 3 else ["schema", "roundtrip"]
t0 = time.perf_counter()
import pydantic  # noqa: E402
from pydantic import (  # noqa: E402
    BaseModel, ConfigDict, Field, TypeAdapter, computed_field, field_serializer,
    field_validator, model_validator,
)
from pydantic.dataclasses import dataclass as pdataclass  # noqa: E402
times = {"import": time.perf_counter() - t0}
assert pydantic.__file__.startswith(sys.argv[1]), pydantic.__file__

T = typing.TypeVar("T")


def define():
    """Defines a fresh set of models (pydantic caches per class, so every call builds
    every core schema and validator again)."""
    class Color(enum.Enum):
        RED = "red"
        GREEN = "green"

    class Address(BaseModel):
        street: str = Field(min_length=1, max_length=100)
        city: str
        zip: str = Field(pattern=r"^\d{5}$")
        country: str = "US"

        @field_validator("city")
        @classmethod
        def title(cls, v: str) -> str:
            return v.title()

    class Tag(BaseModel):
        model_config = ConfigDict(frozen=True)
        name: str
        weight: float = Field(default=1.0, ge=0, le=10)

    class Cat(BaseModel):
        kind: typing.Literal["cat"] = "cat"
        lives: int = Field(default=9, ge=0, le=9)

    class Dog(BaseModel):
        kind: typing.Literal["dog"] = "dog"
        good: bool = True

    class Page(BaseModel, typing.Generic[T]):
        items: list[T]
        total: int = Field(ge=0)
        next: str | None = None

    class User(BaseModel):
        model_config = ConfigDict(str_strip_whitespace=True, validate_assignment=True)
        id: uuid.UUID
        name: str = Field(min_length=1, max_length=50, alias="userName")
        email: str = Field(pattern=r"^[^@]+@[^@]+$")
        age: int | None = Field(default=None, gt=0, lt=150)
        balance: decimal.Decimal = Field(default=decimal.Decimal("0"), max_digits=12,
                                         decimal_places=2)
        created: datetime.datetime
        color: Color = Color.RED
        address: Address
        addresses: list[Address] = []
        tags: set[Tag] = set()
        meta: dict[str, int | str | list[float]] = {}
        pet: Cat | Dog = Field(discriminator="kind")
        friends: list["User"] = []

        @field_validator("name")
        @classmethod
        def no_digits(cls, v: str) -> str:
            if any(c.isdigit() for c in v):
                raise ValueError("digits")
            return v

        @model_validator(mode="after")
        def check(self) -> "User":
            if self.age is not None and self.age < 18 and self.balance > 100:
                raise ValueError("minor with balance")
            return self

        @computed_field
        @property
        def label(self) -> str:
            return f"{self.name} <{self.email}>"

        @field_serializer("balance")
        def ser_balance(self, v: decimal.Decimal) -> str:
            return str(v)

    class Admin(User):
        level: int = Field(default=1, ge=1, le=5)
        perms: tuple[str, ...] = ()

    @pdataclass
    class Point:
        x: float
        y: float = 0.0

    class Shape(BaseModel):
        points: list[Point]
        closed: bool = False

    UserPage = Page[User]
    adapter = TypeAdapter(list[dict[str, list[int]]])
    return User, Admin, UserPage, Shape, adapter


def sample(i):
    return {
        "id": str(uuid.UUID(int=i)), "userName": f" user {chr(97 + i % 26)} ",
        "email": f"u{i}@example.com", "age": 20 + i % 50, "balance": "12.50",
        "created": "2024-01-02T03:04:05", "color": "green",
        "address": {"street": "1 Main St", "city": "springfield", "zip": "12345"},
        "addresses": [{"street": f"{j} Elm", "city": "x", "zip": "54321"} for j in range(3)],
        "tags": [{"name": f"t{j}", "weight": j} for j in range(3)],
        "meta": {"a": 1, "b": "two", "c": [1.0, 2.0]},
        "pet": {"kind": "dog"} if i % 2 else {"kind": "cat", "lives": 3},
        "friends": [{"id": str(uuid.UUID(int=i + 1000)), "userName": "friend",
                     "email": "f@x.y", "created": "2024-01-01T00:00:00",
                     "address": {"street": "s", "city": "c", "zip": "00000"},
                     "pet": {"kind": "cat"}}],
    }


def timed(fn, *args):
    t0 = time.perf_counter()
    for _ in range(REPEAT):
        r = fn(*args)
    return r, (time.perf_counter() - t0) / REPEAT


def schemas(models):
    for m in models[:4]:
        m.model_json_schema()
        m.model_json_schema(mode="serialization")


def roundtrip(models, data, page):
    User, Admin, UserPage, Shape, adapter = models
    users = [User.model_validate(d) for d in data]
    UserPage.model_validate(page)
    dumped = [u.model_dump(mode="json", by_alias=True) for u in users]
    texts = [u.model_dump_json(by_alias=True) for u in users]
    back = [User.model_validate_json(t) for t in texts]
    adapter.validate_python([{"k": [1, 2, 3]}] * 50)
    Shape.model_validate({"points": [{"x": i, "y": i} for i in range(50)]})
    assert back[5] == users[5] and dumped[5]["userName"] == "user f"


def main():
    # `Page[User]` (in define()) walks `sys._getframe(3)` to find its caller; compiled
    # functions have no Python frame, so the compiled tree needs more Python frames
    # above it (docs/DEVIATIONS.md): define() runs three calls deep in both trees.
    models, times["define"] = timed(define)
    if "schema" in PHASES:
        times["schema"] = timed(schemas, models)[1]
    if "roundtrip" in PHASES:
        data = [sample(i) for i in range(200)]
        times["roundtrip"] = timed(roundtrip, models, data, {"items": data[:20], "total": 20})[1]


main()
print(json.dumps(times))
