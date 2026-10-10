def count_primes(limit: int) -> int:
    is_prime = [True] * (limit + 1)
    is_prime[0] = False
    is_prime[1] = False
    i = 2
    while i * i <= limit:
        if is_prime[i]:
            j = i * i
            while j <= limit:
                is_prime[j] = False
                j += i
        i += 1
    count = 0
    for flag in is_prime:
        if flag:
            count += 1
    return count


total = 0
for _ in range(5):
    total += count_primes(2_000_000)
print(total)
