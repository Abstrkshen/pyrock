"""BPE training on a generated corpus: pure Python, so CPython can run it too
(`pyfast bench bench_bpe.py --release` compares the two)."""
from bpe import Tokenizer

SYLLABLES = ["ka", "lo", "mi", "ra", "to", "ne", "su", "vi", "an", "or", "el", "is"]


def corpus(n_words, seed):
    words = []
    x = seed
    for _ in range(n_words):
        x = (x * 1103515245 + 12345) % 2147483648
        n = 1 + x % 4
        w = ""
        for _ in range(n):
            x = (x * 1103515245 + 12345) % 2147483648
            w += SYLLABLES[x % len(SYLLABLES)]
        words.append(w)
    return " ".join(words)


text = corpus(200000, 7)
tok = Tokenizer()
tok.train(text, 120)
ids = tok.encode(text[:5000])
print(len(tok.merges), len(ids), tok.decode(ids[:12]))
