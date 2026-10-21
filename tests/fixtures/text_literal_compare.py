"""`o == ""` next to `o == "0"`: the empty literal's constant has no bytes, so
its address can be the next literal's."""
import re


def parse(t):
    m = re.match(r"^(\d*),?(\d*)$", t)
    out = []
    if m.group(2) == "":
        out.append("empty")
    elif m.group(2) == "0":
        out.append("zero")
    else:
        out.append("other")
    if m.group(1) == "":
        out.append("empty")
    elif m.group(1) == "0":
        out.append("zero")
    return out


for t in ("0,0", "5", ",12", "5,0", "0,12", ""):
    print(t, parse(t))
