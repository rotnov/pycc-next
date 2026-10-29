# Issue #10: wrong calls into `api` (`issue_10_api.py`, as source or as a
# compiled extension module) and missing attributes, each error printed.
import api

g = api.Grid(2, 3)
b = api.Box()
calls = [
    lambda: api.none(1),
    lambda: api.none(1, 2),
    lambda: api.none(k=1),
    lambda: api.one(),
    lambda: api.one(1, 2),
    lambda: api.one(y=1),
    lambda: api.one(1, x=2),
    lambda: api.two(),
    lambda: api.two(1),
    lambda: api.two(1, 2, 3),
    lambda: api.two(y=1),
    lambda: api.two(1, 2, y=3),
    lambda: api.two(1, 2, 3, z=4),
    lambda: api.three(),
    lambda: api.three(1),
    lambda: api.three(1, c=2.0),
    lambda: api.Grid(),
    lambda: api.Grid(1),
    lambda: api.Grid(1, 2, 3),
    lambda: api.Grid(1, h=2, w=3),
    lambda: api.Grid(1, q=2),
    lambda: api.Box(1, 2),
    lambda: api.Box(w=1),
    lambda: g.area(1),
    lambda: g.scale(),
    lambda: g.scale(1, 2),
    lambda: g.scale(b=1),
    lambda: g.resize(),
    lambda: g.resize(1, 2, 3),
    lambda: g.resize(h=2),
    lambda: api.Grid.scale(g),
    lambda: api.Grid.unit(),
    lambda: api.Grid.unit(1, 2),
    lambda: g.unit(1, 2),
    lambda: api.Grid.make(),
    lambda: api.Grid.make(1, 2),
    lambda: b.get(1),
    lambda: g.depth,
    lambda: api.Grid.depth,
    lambda: api.Grid.aera,
    lambda: g.aera,
    lambda: getattr(b, "value"),
    lambda: getattr(api.Box, "value"),
]
for index, call in enumerate(calls):
    try:
        print(index, call())
    except (TypeError, AttributeError) as e:
        print(index, type(e).__name__, e)
g.aera
