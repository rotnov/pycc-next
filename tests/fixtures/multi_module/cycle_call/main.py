"""Calls of functions whose `def` has not run yet, within a module and
across a circular import."""

import registry

print(registry.size())
print(registry.plugins.run(5))
