# A generator expression reading a module-level loop variable the loop
# never bound: NameError, as in CPython.
primes = [2, 3]
for q in range(0):
    pass
print(all(q % p for p in primes))
