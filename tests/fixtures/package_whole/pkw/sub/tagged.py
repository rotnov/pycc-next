"""Subclasses a class of `base` through another module's re-export."""

from ..again import Label


class Bold(Label):
    def show(self) -> str:
        return "**" + self.name + "**"
