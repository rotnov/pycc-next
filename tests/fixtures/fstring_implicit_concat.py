"""Implicit concatenation of plain string literals and f-strings: every
part's text is kept, in order."""

N = 400


def describe(name: str, n: int) -> str:
    return "name=" f"{name!r}" ", n=" f"{n}" "."


def main() -> None:
    print("nested more than the allowed" f" {N} levels")
    print("a" "b" f"{N}" "c" f"-{N + 1}-" "")
    print(f"{N}" "tail\t|")
    print("" f"" "")
    print(describe("x", -3))
    try:
        raise ValueError(
            "TOML inline arrays/tables are nested more than the allowed"
            f" {N} levels"
        )
    except ValueError as e:
        print(e)


main()
