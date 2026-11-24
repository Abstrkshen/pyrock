# mode: native-only
# Print SVG and the playground draws it. It's only text output,
# so the compiled program prints exactly the same picture.
text = """the quick brown fox jumps over the lazy dog the fox
barks and the dog runs while the quick fox hides from the dog"""

counts = {}
for w in text.split():
    counts[w] = counts.get(w, 0) + 1
top = sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))[:8]

W, H, bar = 520, 300, 46
most = top[0][1]
parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" font-family="sans-serif">',
         f'<rect width="{W}" height="{H}" fill="#f8f9f6"/>',
         '<text x="20" y="30" font-size="18" fill="#1c2326">Most common words</text>']
for i, (word, n) in enumerate(top):
    h = round(200 * n / most)
    x = 30 + i * (bar + 16)
    parts.append(f'<rect x="{x}" y="{260 - h}" width="{bar}" height="{h}" rx="4" fill="#0f6e57"/>')
    parts.append(f'<text x="{x + bar // 2}" y="{252 - h}" font-size="13" text-anchor="middle">{n}</text>')
    parts.append(f'<text x="{x + bar // 2}" y="280" font-size="12" text-anchor="middle">{word}</text>')
parts.append("</svg>")
print("\n".join(parts))
