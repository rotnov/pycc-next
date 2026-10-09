def mandel_escape(cx: float, cy: float, max_iter: int) -> int:
    x = 0.0
    y = 0.0
    i = 0
    while i < max_iter:
        x2 = x * x
        y2 = y * y
        if x2 + y2 > 4.0:
            return i
        y = 2.0 * x * y + cy
        x = x2 - y2 + cx
        i = i + 1
    return max_iter

def shade_char(level: int) -> str:
    if level <= 0:
        return " "
    if level == 1:
        return "."
    if level == 2:
        return ":"
    if level == 3:
        return "-"
    if level == 4:
        return "="
    if level == 5:
        return "+"
    if level == 6:
        return "*"
    if level == 7:
        return "#"
    if level == 8:
        return "%"
    return "@"

height = 20
width = 40
max_iter = 20
row = 0
while row < height:
    line = ""
    col = 0
    while col < width:
        cx = 0.0 - 2.0 + (col / width) * 3.0
        cy = 0.0 - 1.0 + (row / height) * 2.0
        iters = mandel_escape(cx, cy, max_iter)
        level = (iters * 9) // max_iter
        line = line + shade_char(level)
        col = col + 1
    print(line)
    row = row + 1
