# mode: native-only
def primes():
    found = []
    n = 2
    while True:
        if all(n % p for p in found):
            found.append(n)
            yield n
        n += 1

def take(it, k):
    out = []
    for x in it:
        if len(out) == k:
            break
        out.append(x)
    return out

print(take(primes(), 10))

words = "the quick brown fox jumps over the lazy dog the end".split()
counts = {}
for w in words:
    counts[w] = counts.get(w, 0) + 1
print(sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))[:3])
