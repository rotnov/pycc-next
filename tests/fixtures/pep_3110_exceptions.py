def risky(kind: int) -> int:
    if kind == 0:
        raise ValueError("value")
    if kind == 1:
        raise KeyError("key")
    return kind


def classify(kind: int) -> str:
    result = "unset"
    try:
        risky(kind)
    except ValueError:
        result = "value"
    except KeyError:
        result = "key"
    else:
        result = "ok"
    finally:
        print("cleanup")
    return result


def reraise(kind: int) -> str:
    try:
        risky(kind)
    except ValueError:
        print("inner handler")
        raise
    return "no exception"


print(classify(0))
print(classify(1))
print(classify(2))

print(reraise(2))

try:
    print(reraise(0))
except ValueError:
    print("outer handler")

try:
    print("body runs")
finally:
    print("finally always runs")
class AppError(Exception):
    pass


class DatabaseError(AppError):
    pass


class ConfigError(AppError):
    pass


def load(kind: int) -> str:
    if kind == 0:
        raise DatabaseError("connection refused")
    if kind == 1:
        raise ConfigError("missing key")
    if kind == 2:
        raise AppError("generic")
    return "loaded"


def attempt(kind: int) -> str:
    try:
        return load(kind)
    except DatabaseError:
        return "database"
    except AppError:
        return "app"


print(attempt(0))
print(attempt(1))
print(attempt(2))
print(attempt(3))

try:
    raise DatabaseError("root reached")
except Exception:
    print("root handler")

try:
    try:
        raise ConfigError("propagated")
    finally:
        print("inner finally")
except AppError:
    print("outer handler")

try:
    raise DatabaseError("chained") from ConfigError("cause")
except AppError:
    print("chained handler")
