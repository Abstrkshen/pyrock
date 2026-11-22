import geo
import sig
from geo import Counter


def square(x):
    return x * x


xs = [1.0, 2.0, 4.0]
print(geo.mean(xs), geo.mean([]))
geo.scale_all(xs, 10.0)
print(xs)
ids = [1]
geo.push_twice(ids, 7)
print(ids)
words = geo.split_words("the cat the hat")
print(words, geo.min_max([3, -1, 8]))
h = geo.histogram(words)
print(sorted(h.items()))
offset = 100
print(geo.apply([1, 2, 3], square), geo.apply([1, 2], lambda v: v + offset))
print(geo.count_if([0.5, 2.0, 3.5], lambda v: v > 1.0), geo.format_int(-12345))
c = Counter("hits")
c.incr(2)
c.incr(3)
d = geo.Counter.new("misses")
d.count = 4
print(c.label(), d.label(), c.count, d.name, geo.total(c, d))
print(sig.dot([3.0, 4.0]))
data = [2.0, 8.0, 4.0]
sig.normalize(data)
print(data)
v = sig.vadd(sig.Vec2(1.0, 2.0), sig.Vec2(0.5, 0.5))
print(v.x, v.y, sig.brightness(sig.Color(30, 60, 90)))
print(geo.checksum(b"abc"), geo.checksum([1, 2, 3]), geo.shout("hi"))
print(round(sig.integrate(square, 0.0, 3.0, 1000), 4))
