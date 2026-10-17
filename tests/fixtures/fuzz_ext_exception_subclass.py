class AppError(Exception):
    pass


class CodeError(AppError):
    def __init__(self, code: int) -> None:
        super().__init__(f"code {code}")
        self.code = code


def risky(n: int) -> int:
    if n < 0:
        raise CodeError(n)
    if n == 0:
        raise AppError("zero")
    return n * 2


def total() -> int:
    out = 0
    for v in [3, -4, 0, 5]:
        try:
            out += risky(v)
        except AppError:
            out += 100
        except ValueError:
            out -= 1
    return out


def message(n: int) -> str:
    try:
        risky(n)
    except AppError as e:
        return str(e)
    return "fine"


print(total(), message(-2), message(0), message(1))
risky(-7)
