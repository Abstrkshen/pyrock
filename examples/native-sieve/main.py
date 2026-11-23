# pyrock run main.py
# pyrock check main.py --mode native-only

def is_prime(n):
    if n < 2:
        return False
    d = 2
    while d * d <= n:
        if n % d == 0:
            return False
        d = d + 1
    return True


def primes_below(limit):
    line = ""
    n = 2
    while n < limit:
        if is_prime(n):
            line = line + str(n) + " "
        n = n + 1
    return line


def count_primes(limit):
    n = 2
    count = 0
    while n < limit:
        if is_prime(n):
            count = count + 1
        n = n + 1
    return count


print(primes_below(80))
print("count below 3000:", count_primes(3000))
