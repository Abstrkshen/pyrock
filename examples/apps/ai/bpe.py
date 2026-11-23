"""Byte-pair encoding: learn merges from text, then encode and decode.
Plain Python lists, dicts and tuples, compiled natively."""


def pair_counts(words):
    counts = {}
    for word, freq in words.items():
        for i in range(len(word) - 1):
            pair = (word[i], word[i + 1])
            counts[pair] = counts.get(pair, 0) + freq
    return counts


def merge_word(word, pair, new_id):
    out = []
    i = 0
    while i < len(word):
        if i < len(word) - 1 and word[i] == pair[0] and word[i + 1] == pair[1]:
            out.append(new_id)
            i += 2
        else:
            out.append(word[i])
            i += 1
    return tuple(out)


class Tokenizer:
    def __init__(self):
        self.merges = []
        self.vocab = {i: chr(i) for i in range(256)}

    def train(self, text, num_merges):
        words = {}
        for w in text.split():
            key = tuple(ord(c) for c in w + " ")
            words[key] = words.get(key, 0) + 1
        for _ in range(num_merges):
            counts = pair_counts(words)
            if not counts:
                break
            best = max(counts.items(), key=lambda kv: (kv[1], -kv[0][0], -kv[0][1]))[0]
            new_id = 256 + len(self.merges)
            self.merges.append(best)
            self.vocab[new_id] = self.vocab[best[0]] + self.vocab[best[1]]
            words = {merge_word(w, best, new_id): f for w, f in words.items()}

    def encode(self, text):
        ids = []
        for w in text.split():
            word = tuple(ord(c) for c in w + " ")
            for i, pair in enumerate(self.merges):
                word = merge_word(word, pair, 256 + i)
            ids.extend(word)
        return ids

    def decode(self, ids):
        return "".join(self.vocab[i] for i in ids).strip()
