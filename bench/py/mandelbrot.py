def mandel(width: int, height: int, max_iter: int) -> int:
    inside = 0
    for py in range(height):
        for px in range(width):
            cr = 3.0 * px / width - 2.0
            ci = 2.0 * py / height - 1.0
            zr = 0.0
            zi = 0.0
            n = 0
            while n < max_iter and zr * zr + zi * zi <= 4.0:
                t = zr * zr - zi * zi + cr
                zi = 2.0 * zr * zi + ci
                zr = t
                n += 1
            if n == max_iter:
                inside += 1
    return inside


print(mandel(600, 400, 200))
