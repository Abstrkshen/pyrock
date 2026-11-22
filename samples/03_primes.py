# Integer loops with early exit: trial-division primality and prime counting.
def is_prime(n):
    if n < 2:
        return False
    if n % 2 == 0:
        return n == 2
    d = 3
    while d * d <= n:
        if n % d == 0:
            return False
        d += 2
    return True


def count_primes(limit):
    count = 0
    for k in range(limit):
        if is_prime(k):
            count += 1
    return count


def nth_prime(n):
    found = 0
    k = 1
    while found < n:
        k += 1
        if is_prime(k):
            found += 1
    return k


print("primes below 100000:", count_primes(100000))
print("the 1000th prime is", nth_prime(1000))
print("is 2147483647 prime?", is_prime(2147483647))
