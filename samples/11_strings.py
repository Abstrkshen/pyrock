# Native strings (concat, repeat, compare, len, in) plus str methods via CPython.
def banner(title, width):
    pad = (width - len(title) - 2) // 2
    return "=" * pad + " " + title + " " + "=" * pad


def is_palindrome(s):
    cleaned = s.lower().replace(" ", "")
    return cleaned == cleaned[::-1]


def caesar(text, shift):
    out = ""
    for ch in text:
        if "a" <= ch <= "z":
            out = out + chr((ord(ch) - ord("a") + shift) % 26 + ord("a"))
        else:
            out = out + ch
    return out


print(banner("pyfast", 30))
print("abc" < "abd", "Zebra" < "apple", "fast" in "pyfast", len("héllo"))
print(is_palindrome("A man a plan a canal Panama"), is_palindrome("pyfast"))
secret = caesar("hello world", 3)
print(secret, "->", caesar(secret, 23))
print("-".join(["a", "b", "c"]), "x,y,z".split(","), "  trim  ".strip())
print(f"{3.14159:.2f} | {42:>6} | {'left':<8}|")
