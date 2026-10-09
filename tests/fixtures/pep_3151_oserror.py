def raise_file_not_found() -> None:
    raise FileNotFoundError("missing")


def raise_broken_pipe() -> None:
    raise BrokenPipeError("pipe gone")


def raise_timeout() -> None:
    raise TimeoutError("too slow")


def classify_os_error(kind: int) -> str:
    result = "unset"
    try:
        if kind == 0:
            raise_file_not_found()
        elif kind == 1:
            raise_broken_pipe()
        else:
            raise OSError("generic")
    except FileNotFoundError:
        result = "file not found"
    except ConnectionError:
        result = "connection"
    except OSError:
        result = "os"
    return result


def catch_connection_child_via_connection_error() -> str:
    try:
        raise_broken_pipe()
    except ConnectionError:
        return "caught via ConnectionError"
    return "unreached"


def catch_deep_child_via_os_error() -> str:
    try:
        raise_broken_pipe()
    except OSError:
        return "caught via OSError"
    return "unreached"


def sibling_does_not_catch() -> str:
    try:
        raise_timeout()
    except ConnectionError:
        return "wrongly caught by ConnectionError"
    except TimeoutError:
        return "caught via TimeoutError"
    return "unreached"


def finally_runs_on_the_exceptional_path() -> str:
    try:
        try:
            raise_file_not_found()
        finally:
            print("inner finally")
    except OSError:
        return "outer caught"
    return "unreached"


print(classify_os_error(0))
print(classify_os_error(1))
print(classify_os_error(2))
print(catch_connection_child_via_connection_error())
print(catch_deep_child_via_os_error())
print(sibling_does_not_catch())
print(finally_runs_on_the_exceptional_path())
