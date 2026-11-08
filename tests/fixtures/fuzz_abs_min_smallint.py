# Found by the differential fuzzer (fuzz/): abs() of the most negative
# smallint, -2**62, is 2**62, one past the smallint range, so it must take
# the bigint path even where the argument's range is known at compile time.
print(f' {abs(-4611686018427387904):}')
print(abs(-4611686018427387904), abs(-4611686018427387903), abs(4611686018427387903))
x = -4611686018427387904
y = abs(x)
print(y, y + 1, y - 1, y * 2, -y)
for i in range(-4611686018427387904, -4611686018427387901):
    print(i, abs(i), abs(i) - 4611686018427387904)
z = -4611686018427387903 if len(str(x)) > 3 else -4611686018427387904
print(abs(z), abs(z - 1))
