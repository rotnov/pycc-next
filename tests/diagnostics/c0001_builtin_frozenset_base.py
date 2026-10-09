class fzset(frozenset):
    def __repr__(self) -> str:
        return "{%s}" % ", ".join(map(repr, self))


def size(items: fzset) -> int:
    return len(items)
