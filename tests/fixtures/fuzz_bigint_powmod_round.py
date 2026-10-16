# Found by the differential fuzzer (fuzz/): three-argument `pow()` with a
# bigint operand and `round()` of a float beyond 63 bits raised
# "not supported yet" OverflowErrors.


def powmod(a: int, b: int, m: int) -> None:
    try:
        print(pow(a, b, m))
    except ValueError as e:
        print("ValueError", e)


powmod(9223372036854775807, 0, 1)
powmod(50000000000000000000, 3, 7)
powmod(7, 2**70, 10**30 + 7)
powmod(-5, 2**70, -50000000000000000000)
powmod(3, -1, 50000000000000000000)
powmod(2**64, -1, 4)
powmod(10**25, -3, 10**18 + 9)
powmod(-(10**20), 5, 10**21)
powmod(10**20, 7, 0)
print(round(1e300) == int(1e300), round(-2.5e19), round(9.3e18), round(1.7976931348623157e308) % 1000)
print(round(2.0**63), round(-(2.0**63)), round(1e19 + 0.0))
