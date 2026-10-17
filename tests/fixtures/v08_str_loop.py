"""v0.8 accept 4: `for c in s` over a `str` reads one character per trip:
early `break`, `continue`, `else`, rebinding the string inside the loop,
nested loops, non-ASCII text, the empty string, the loop variable after the
loop."""


def count_indent(text: str) -> int:
    column = 0
    for c in text:
        if c == " ":
            column += 1
        elif c == "\t":
            column += 4 - column % 4
        else:
            break
    return column


def vowels(text: str) -> str:
    out = ""
    for c in text:
        if c not in "aeiouéö":
            continue
        out += c
    return out


def first_digit(text: str) -> str:
    for c in text:
        if c.isdigit():
            found = c
            break
    else:
        found = "-"
    return found


def rebind(text: str) -> str:
    seen = ""
    for c in text:
        seen += c
        text = "zzz"
    return seen + "|" + text


def pairs(a: str, b: str) -> int:
    n = 0
    for x in a:
        for y in b:
            if x == y:
                n += 1
    return n


def last_char(text: str) -> str:
    c = "?"
    for c in text:
        pass
    return c


def main() -> None:
    for t in ["", "x", "    four", "\t\tab", "  \t x", "héllö wörld", "abc1def2", "€" * 30]:
        print(count_indent(t), repr(vowels(t)), first_digit(t), rebind(t), pairs(t, "loö"),
              repr(last_char(t)))
    print(count_indent(" " * 10000 + "x"))
    s = "ab"
    print([c + c for c in s], list(s))
    print(last_char("abc")[3])


main()
