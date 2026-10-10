# The CPython side of tests/object_operand_pin.rs. Every hook below calls
# `nest()`, which runs the module under test again beneath the caller: a
# second exec of a compiled module shares its global slots, so the nested
# body rebinds -- and releases -- every global the outer operation is
# using. Each hook then keeps using its operands, and CPython itself
# keeps using them after the hook returns (the reflected operator, the
# dict probe after `__hash__`).
import importlib
import sys

NEST = None
DEPTH = 0
LIVE = 0


def nest():
    global DEPTH
    if NEST is None or DEPTH:
        return
    DEPTH += 1
    outer = sys.modules[NEST]
    del sys.modules[NEST]
    try:
        importlib.import_module(NEST)
    finally:
        sys.modules[NEST] = outer
        DEPTH -= 1


class Counted:
    def __init__(self, tag):
        global LIVE
        LIVE += 1
        self.tag = tag
        self.data = [tag, tag]

    def __del__(self):
        global LIVE
        if LIVE is not None:
            LIVE -= 1

    def __repr__(self):
        return type(self).__name__ + "(" + self.tag + str(len(self.data)) + ")"


class Item(Counted):
    # The right-hand operand: CPython calls its reflected methods after the
    # left operand's hook has run.
    def __eq__(self, other):
        return "Item.eq " + repr(self)

    def __radd__(self, other):
        return "Item.radd " + repr(self)

    __hash__ = None


class Nester(Counted):
    def __eq__(self, other):
        nest()
        return NotImplemented

    def __add__(self, other):
        nest()
        return NotImplemented

    def __neg__(self):
        nest()
        return "neg " + repr(self)

    def __getattr__(self, name):
        nest()
        return name + " of " + repr(self)

    def method(self):
        nest()
        return "method of " + repr(self)

    def __call__(self, n):
        nest()
        return "call " + str(n) + " " + repr(self)

    def __len__(self):
        nest()
        return len(self.data)

    def __getitem__(self, index):
        nest()
        return self.data[index] + repr(self)

    def __iter__(self):
        nest()
        return iter(self.data + [repr(self)])

    def __format__(self, spec):
        nest()
        return "format " + repr(self)

    __hash__ = None


class Key(Counted):
    # A dict probes its table after hashing the key.
    def __hash__(self):
        nest()
        return 1

    def __eq__(self, other):
        return isinstance(other, Key)


class Rep(Counted):
    def __repr__(self):
        nest()
        return "Rep(" + self.tag + ")"


def make_dict():
    return {Key("stored"): Item("value")}


def make_list():
    return [Rep("a"), Rep("b")]
