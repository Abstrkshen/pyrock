"""Train a BPE tokenizer (Python, compiled natively), count bigrams in a
Rust model and pick tokens with C sampling helpers."""
import ngram
import sampling
from bpe import Tokenizer

CORPUS = """
the quick brown fox jumps over the lazy dog . the lazy dog sleeps in the sun .
a quick brown dog runs over the hill . the fox and the dog are friends .
the sun is warm and the hill is green . the brown fox runs in the green grass .
a lazy fox sleeps under the warm sun . the quick dog jumps over the brown fox .
"""


def generate(model, tok, start, steps):
    ids = [start]
    for _ in range(steps):
        probs = model.logits(ids[-1], 0.1)
        sampling.softmax(probs)
        ids.append(sampling.argmax(probs))
    return ids


tok = Tokenizer()
tok.train(CORPUS * 4, 40)
ids = tok.encode(CORPUS)
print("merges:", len(tok.merges), "first:", [tok.vocab[256 + i] for i in range(5)])
print("tokens:", len(ids), "of", len(CORPUS.split()), "words; ratio", round(len(ids) / len(CORPUS.replace(" ", "")), 3))
print("roundtrip:", tok.decode(tok.encode("the lazy fox")) == "the lazy fox")

model = ngram.Bigram.new(256 + len(tok.merges))
model.train(ids)
print("pairs:", model.pairs(), "vocab:", model.vocab)
print("perplexity:", round(ngram.perplexity(model, ids, 0.1), 2))

start = tok.encode("the")[0]
out = generate(model, tok, start, 12)
print("generated:", tok.decode(out))
probs = model.logits(start, 0.1)
sampling.softmax(probs)
print("entropy after 'the':", round(sampling.entropy(probs), 3), "bits")
