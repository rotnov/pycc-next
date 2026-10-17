"""v0.8 accept 4: in non-strict mode a `typing` name pycc does not model is
the CPython object it names, and an annotation naming it compiles."""

import re
from typing import Any, Callable, Match, MutableMapping, Optional, Pattern

WORD: Pattern[str] = re.compile(r"\w+")


def word_end(m: Match[str]) -> int:
    return m.end()


def count_words(text: str, env: MutableMapping[str, Any]) -> int:
    n = 0
    pos = 0
    while True:
        m: Optional[Match[str]] = WORD.search(text, pos)
        if m is None:
            break
        env[m.group(0)] = env.get(m.group(0), 0) + 1
        pos = word_end(m)
        n += 1
    return n


def apply(f: Callable[[str], str], s: str) -> str:
    return f(s)


env: dict[str, int] = {}
print(count_words("a bb a ccc bb a", env), sorted(env.items()))
print(apply(str.upper, "shout"), Match is re.Match, Pattern is re.Pattern)
print(word_end(WORD.match("hello world")))
print(word_end(None))
