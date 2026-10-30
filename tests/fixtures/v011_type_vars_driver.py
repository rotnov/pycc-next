"""Imports `api` and prints the type variables its import made; the compiled
module must make as many as its source, and compiled code must see the one
Python code sees."""
import gc
import typing

gc.collect()
before = {id(o) for o in gc.get_objects() if isinstance(o, typing.TypeVar)}
import api

gc.collect()
made = [o for o in gc.get_objects() if isinstance(o, typing.TypeVar) and id(o) not in before]
print("made", sorted(o.__name__ for o in made))
print(api.T, api.U, api.Box.__parameters__, api.Box[int])
print(api.T is api.Box.__parameters__[0], api.type_var() is api.T)
print(api.first([3, 4]), api.Box("v").get())
