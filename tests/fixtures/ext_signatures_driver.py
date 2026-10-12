import ext_signatures as m

calls = [
    lambda: m.add(1),
    lambda: m.add(1, 2),
    lambda: m.add(a=3, b=4),
    lambda: m.add(b=5, a=6),
    lambda: m.scale(3.0),
    lambda: m.scale(3.0, factor=0.5),
    lambda: m.total(),
    lambda: m.total(1, 2, 3),
    lambda: m.total(*range(5)),
    lambda: m.tags("n"),
    lambda: m.tags("n", k="v", z="w"),
    lambda: m.tags("n", "+", k="v"),
    lambda: m.mix(1, 2, 3, 4, c=5),
    lambda: m.mix(1, b=2, c=3, d=0, e=9),
    lambda: m.pos(5),
    lambda: m.pos(5, b=2),
    lambda: m.shout("a", "b"),
    lambda: m.Box().area(),
    lambda: m.Box(3, 4).area(scale=2),
    lambda: m.Box(w=2).grow(1, 2),
    lambda: m.Box(2).grow(),
    lambda: m.use_inside(),
    lambda: m.add(),
    lambda: m.add(1, 2, 3),
    lambda: m.add(1, c=2),
    lambda: m.add(1, a=2),
    lambda: m.scale(1.0, 2.0),
    lambda: m.mix(1, 2),
    lambda: m.mix(a=1, b=2, c=3),
    lambda: m.pos(a=5),
    lambda: m.Box().area(3),
    lambda: m.Box().area(scale=1, x=2),
    lambda: m.tags(),
]
for call in calls:
    try:
        print(call())
    except TypeError as e:
        print("TypeError:", e)
print(m.add.__name__, m.add.__qualname__, m.Box.area.__name__, m.add.__module__)
