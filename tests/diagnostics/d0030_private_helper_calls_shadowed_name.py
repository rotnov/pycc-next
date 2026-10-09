def helper() -> int:
    return 1

helper = 2

def _call_helper():
    return helper()

print(_call_helper())
