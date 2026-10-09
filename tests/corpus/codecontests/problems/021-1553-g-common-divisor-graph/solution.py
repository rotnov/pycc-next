import itertools
from sys import stdin


def input():
    return next(stdin)[:-1]


def readline():
    return map(int, input().split())


primes = [2]
for q in range(3, 10**3, 2):
    if all(q % p for p in primes):
        primes.append(q)


def prime_divisors(n):
    for p in primes:
        if p * p > n:
            break
        if n % p == 0:
            yield p
            while n % p == 0:
                n //= p
    if n > 1:
        yield n


def test_prime_divisors():
    assert list(prime_divisors(6)) == [2, 3]
    assert list(prime_divisors(9)) == [3]
    assert list(prime_divisors(10)) == [2, 5]
    assert list(prime_divisors(7)) == [7]
    assert list(prime_divisors(997)) == [997]
    assert list(prime_divisors(1009)) == [1009]


class DSU:
    def __init__(self):
        self.p = list(range(10**6))

    def _get_root(self, a):
        while self.p[a] != a:
            a = self.p[a]
        return a

    def __setitem__(self, a, b):
        assert b <= a
        r = self._get_root(b)
        while a != r:
            self.p[a], a = r, self.p[a]

    def __getitem__(self, a):
        self[a] = a
        return self.p[a]


def main():
    n, q = readline()
    a = list(readline())
    queries = (readline() for __ in range(q))

    min_divisor = list()
    dsu = DSU()
    for ai in a:
        p, *rest = prime_divisors(ai)
        min_divisor.append(p)
        for q in rest:
            dsu[q] = p

    close = set()
    for (ai, p) in zip(a, min_divisor):
        group = sorted({dsu[p]} | {dsu[q] for q in prime_divisors(ai + 1)})
        close.update(itertools.combinations(group, 2))
    for (p, q) in close:
        assert p < q

    for (s, t) in queries:
        p = dsu[min_divisor[s-1]]
        q = dsu[min_divisor[t-1]]
        p, q = sorted((p, q))
        if p == q:
            print(0)
        elif (p, q) in close:
            print(1)
        else:
            print(2)


if __name__ == '__main__':
    main()


# t.me/belkka
