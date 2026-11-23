#!/usr/bin/env bash
# Smoke-test a pyrock binary before it is released.
#
#   bash scripts/release_smoke.sh PATH/TO/pyrock
#
# Compiles and runs, from an empty directory: a fully native program (no CPython), and a
# hybrid program that imports a CPython module, a Rust file and a C file. Failures are
# printed as GitHub Actions error annotations, so they show on the run's summary page.
set -euo pipefail

bin=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
cd "$dir"

fail() {
    # One line: annotations don't keep newlines.
    echo "::error title=release smoke test::$1 | $(tail -c 600 err.txt 2>/dev/null | tr '\n' ' ')"
    exit 1
}

"$bin" --version

cat > fib.py <<'EOF'
def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)

print(fib(25))
EOF
out=$("$bin" run fib.py --mode native-only 2>err.txt) || fail "native-only fib.py did not build or run"
[ "$out" = "75025" ] || fail "native-only fib.py printed '$out', expected 75025"
echo "native-only: ok"

cat > hybrid.py <<'EOF'
import json
import vec
import fastc

data = json.loads('{"xs": [1, 2, 3]}')
total = 0
for x in data["xs"]:
    total += x
print(total, vec.dot(1.5, 2.0, 3.0, 4.0), fastc.gcd(1071, 462))
EOF
printf 'pub fn dot(a: f64, b: f64, c: f64, d: f64) -> f64 {\n    a * c + b * d\n}\n' > vec.rs
printf 'int gcd(int a, int b) {\n    while (b) { int t = a %% b; a = b; b = t; }\n    return a;\n}\n' > fastc.c
out=$("$bin" run hybrid.py 2>err.txt) || fail "hybrid.py (json + Rust + C imports) did not build or run"
[ "$out" = "6 12.5 21" ] || fail "hybrid.py printed '$out', expected '6 12.5 21'"
echo "hybrid with Rust and C imports: ok"

"$bin" explain hybrid.py > /dev/null 2>err.txt || fail "explain failed"
echo "explain: ok"
