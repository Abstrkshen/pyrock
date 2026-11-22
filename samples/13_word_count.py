# Text processing with collections.Counter and sorted (a C-accelerated stdlib).
from collections import Counter

text = """the quick brown fox jumps over the lazy dog the fox
barks and the dog sleeps while the quick fox runs"""

words = text.split()
counts = Counter(words)
print("words:", len(words), "distinct:", len(counts))
for word in sorted(counts):
    if counts[word] > 1:
        print(word.rjust(6), "*" * counts[word])
print("top 3:", counts.most_common(3))
