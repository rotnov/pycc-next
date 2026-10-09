# PEP 701 formalized the f-string grammar in 3.12: the interpolation body is
# now parsed as ordinary Python, so it may reuse the enclosing quote
# character, span multiple lines, carry comments, and contain backslash
# escapes -- all of which were syntax errors before.
#
# Conversion flags (`!r`/`!s`/`!a`), format specs (`{x:...}`) and the `=`
# debug specifier are deliberately absent: they are recorded core gaps in
# pycc's f-string lowering, not PEP 701 grammar points. All three are
# rejected outright; #720 tracks the `=` debug specifier.


def nested_same_quote(x: int) -> str:
    # The PEP's headline change: the inner f-string reuses `"` inside a
    # `"`-delimited interpolation.
    return f"{f"{x}"}"


def deep_nesting(a: int) -> str:
    return f"{f"{f"{f"{f"{a}"}"}"}"}"


def single_quotes_inside(a: int) -> str:
    return f"{f'{a}'}"


def all_single_quote(a: int) -> str:
    return f'{f'{a}'}'


def multiline_interp(a: int, b: int) -> str:
    # The interpolation expression spans several physical lines.
    return f"""{
        a
        +
        b
    }"""


def commented_interp(a: int, b: int) -> str:
    # A `#` comment inside a multi-line interpolation.
    return f"""{
        a  # the left operand
        + b  # the right operand
    }"""


def named_escape_in_interp(a: int) -> str:
    # A `\N{...}` named escape inside the interpolation body. The names used
    # here resolve to ASCII characters so the fixture's stdout stays ASCII on
    # every Tier-1 target.
    return f"{"\N{NUMBER SIGN}"}{a}"


def named_escape_in_literal(a: int) -> str:
    return f"\N{DOLLAR SIGN}{a}\N{FULL STOP}"


def adjacent_interpolations(n: int) -> str:
    return f"{n}{n}{n}"


def escaped_braces(n: int) -> str:
    return f"{{{n}}}"


def raw_fstring(n: int) -> str:
    return rf"a\tb{n}"


def implicit_concat(n: int) -> str:
    return f"x{n}" f"y{n}"


print(nested_same_quote(7))
print(deep_nesting(11))
print(single_quotes_inside(13))
print(all_single_quote(17))
print(multiline_interp(2, 3))
print(commented_interp(4, 5))
print(named_escape_in_interp(1))
print(named_escape_in_literal(2))
print(adjacent_interpolations(3))
print(escaped_braces(4))
print(raw_fstring(5))
print(implicit_concat(6))
