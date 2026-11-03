import m

print(m.get(), m.name())
m.X = 10
print(m.get(), m.name())
m.NAME = "c"
m.X = 11
print(m.get(), m.name())
vars(m)["X"] = 12
print(m.get())
