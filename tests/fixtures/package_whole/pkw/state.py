print("init state")
COUNTER = 0
LIMIT = 3


def bump() -> int:
    global COUNTER
    COUNTER += 1
    return COUNTER


def limit() -> int:
    return LIMIT
