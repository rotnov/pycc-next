try:
    raise ValueError("bad")
except* ValueError:
    print("single caught")

try:
    raise ValueError("bad")
except* TypeError:
    print("wrong type")
except* ValueError:
    print("dispatched value")

try:
    raise ValueError("v")
except ValueError as e1:
    try:
        raise TypeError("t")
    except TypeError as e2:
        try:
            raise ExceptionGroup("multi", [e1, e2])
        except* ValueError:
            print("group caught value")
        except* TypeError:
            print("group caught type")

try:
    raise ValueError("bad")
except* ValueError as eg:
    saved = eg
    print("as binding caught")

try:
    raise ValueError("bad")
except* ValueError:
    print("finally handler")
finally:
    print("finally cleanup")

try:
    print("else body")
except* ValueError:
    print("else handler")
else:
    print("else ran")
