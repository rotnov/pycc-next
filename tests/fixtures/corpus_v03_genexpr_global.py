# A generator expression reading a module-level loop variable, and
# `any`/`all` of a generator of CPython objects (non-strict mode).
primes = [2]
for q in range(3, 60, 2):
    if all(q % p for p in primes):
        primes.append(q)
print(primes)
mixed = [0, "", "x", None, [1]]
print(any(v for v in mixed), all(v for v in mixed), all(v for v in mixed[2:3]))
print(any(v for v in mixed[:2]), all(v for v in mixed[4:]))
