def fib_recursive(n: int) -> int:
    if n < 2:
        return n
    return fib_recursive(n - 1) + fib_recursive(n - 2)

def fib_iterative(n: int) -> int:
    a = 0
    b = 1
    i = 0
    while i < n:
        temp = a + b
        a = b
        b = temp
        i = i + 1
    return a

i = 0
while i < 11:
    print(fib_recursive(i))
    i = i + 1

print(fib_iterative(100))
