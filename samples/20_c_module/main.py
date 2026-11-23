# Importing a C file: signatures come from clib.h, the code is compiled with cc.
import clib


def checksum_of(word):
    return clib.checksum(word)


print("gcd:", clib.gcd(1071, 462), clib.gcd(2 ** 40, 2 ** 20))
print("dot3:", clib.dot3(1.0, 2.0, 3.0, 4.0, 5.0, 6.0))
print("checksum:", checksum_of("pyrock"), checksum_of(""))
print("is_even:", clib.is_even(10), clib.is_even(7))
print("lib:", clib.version())
print("int too big for C int:", clib.add_int(2 ** 31, 1))
