"""Two project modules import each other (a circular import)."""

import shapes

print(shapes.total(), shapes.start())
print(shapes.describe(4))
