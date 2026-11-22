from array import array
import kern

a = array("d", [1.0, 2.0, 3.0])
kern.scale_all(a, 2.0)
print(a, kern.total(a))
print(kern.checksum(b"abc"), kern.checksum(bytearray(b"ab")))
secret = kern.xor(b"hello", 7)
print(secret, kern.xor(secret, 7))
