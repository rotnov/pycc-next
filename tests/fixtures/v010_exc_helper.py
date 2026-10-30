"""An `Exception` that is not the builtin: its `code` is a property
(`tests/fixtures/v010_exc_shadow_builtin.py`)."""
import builtins


class Exception(builtins.Exception):
    @property
    def code(self):
        return 42

    @code.setter
    def code(self, value):
        self.stored = value
