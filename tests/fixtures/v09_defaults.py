# v0.9: class-level `None` defaults that instances shadow
# (`pycc_ast::desugar::instance_defaults`): an instance slot initialized to
# the default, the class keeping the attribute too.


class Spec(object):
    """A spec whose attributes default to None."""

    template = None
    encoding = None


class Grid:
    label = None

    def __init__(self, n: int) -> None:
        self.n = n

    def show(self) -> str:
        return str(self.label) + "/" + str(self.n)

    def relabel(self, label: str) -> None:
        self.label = label


def describe(s: Spec) -> str:
    return str(s.template) + ":" + str(s.encoding)


g = Grid(2)
print(g.show(), Grid.label)
g.relabel("L")
print(g.show())
s = Spec()
print(describe(s), Spec.template)
