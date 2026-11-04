import threading

ev = {k: threading.Event() for k in ("a in", "b in", "a go", "b go", "a done")}


def hold(name):
    ev[name + " in"].set()
    ev[name + " go"].wait(30)
