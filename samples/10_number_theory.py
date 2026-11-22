# Integer semantics that differ from C/Rust: floor division, modulo sign, pow.
def gcd(a, b):
    while b != 0:
        t = a % b
        a = b
        b = t
    return abs(a)


def lcm(a, b):
    return a // gcd(a, b) * b


def mod_pow(base, exp, m):
    result = 1
    base = base % m
    while exp > 0:
        if exp % 2 == 1:
            result = result * base % m
        exp = exp // 2
        base = base * base % m
    return result


def digit_sum(n):
    s = 0
    while n > 0:
        s += n % 10
        n //= 10
    return s


print(gcd(1071, 462), gcd(-12, 18), lcm(21, 6))
print(mod_pow(2, 1000, 1000000007), mod_pow(3, 200, 13))
print(-7 // 2, -7 % 2, 7 // -2, 7 % -2, 2 ** 62)
print(digit_sum(2 ** 50), 1 << 40, -1 >> 3, 0b1010 ^ 0b0110)
