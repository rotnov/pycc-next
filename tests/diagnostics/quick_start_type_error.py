def fib(n: int) -> int:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)

i = 0
while i < 11:
    print(fib(i))
    i = i + 1
print(fib("5"))
