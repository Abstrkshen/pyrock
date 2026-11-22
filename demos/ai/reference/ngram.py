"""CPython reference for ngram.rs (only used to check the demo's output)."""
import math


class Bigram:
    def __init__(self, vocab):
        self.vocab = vocab
        self.counts = {}
        self.totals = {}

    @staticmethod
    def new(vocab):
        return Bigram(vocab)

    def train(self, ids):
        for a, b in zip(ids, ids[1:]):
            self.counts[(a, b)] = self.counts.get((a, b), 0.0) + 1.0
            self.totals[a] = self.totals.get(a, 0.0) + 1.0

    def logits(self, prev, k):
        total = self.totals.get(prev, 0.0) + k * self.vocab
        return [math.log((self.counts.get((prev, t), 0.0) + k) / total) for t in range(self.vocab)]

    def pairs(self):
        return len(self.counts)


def perplexity(model, ids, k):
    nll = 0.0
    for a, b in zip(ids, ids[1:]):
        nll -= model.logits(a, k)[b]
    return math.exp(nll / (len(ids) - 1))
