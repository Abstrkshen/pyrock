def make_text(n):
    words = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"]
    out = []
    for i in range(n):
        out.append(words[(i * 7 + i // 3) % len(words)])
    return " ".join(out)


def count_words(text):
    counts = {}
    for w in text.split():
        counts[w] = counts.get(w, 0) + 1
    return counts


def top(counts, k):
    pairs = sorted([(c, w) for w, c in counts.items()], reverse=True)
    return pairs[:k]


def primes(n):
    sieve = [True] * (n + 1)
    sieve[0] = False
    sieve[1] = False
    i = 2
    while i * i <= n:
        if sieve[i]:
            for j in range(i * i, n + 1, i):
                sieve[j] = False
        i += 1
    return [i for i in range(n + 1) if sieve[i]]


text = make_text(300000)
c = count_words(text)
print(top(c, 3))
p = primes(2000000)
print(len(p), p[-1], sum(p[:1000]))
