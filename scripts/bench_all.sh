#!/usr/bin/env bash
# Measure every benchmark the docs quote, against CPython, and print the table.
#
#   bash scripts/bench_all.sh [--runs N] [--gate] [PATH/TO/pyrock]
#
# Prints a Markdown table (with the commit it measured) to paste into docs/status.md.
# With --gate, also fails when a speedup is below its floor: floors sit well under the
# documented figures, so a CI runner's noise passes and a real regression doesn't.
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD

runs=3
gate=0
bin="$ROOT/target/release/pyrock"
while [ $# -gt 0 ]; do
    case "$1" in
        --runs) runs=$2; shift 2 ;;
        --gate) gate=1; shift ;;
        *) bin=$(cd "$(dirname "$1")" && pwd)/$(basename "$1"); shift ;;
    esac
done
[ -x "$bin" ] || { echo "no pyrock binary at $bin (cargo build --release -p pyrock)" >&2; exit 2; }

# file | what it does | floor for --gate
BENCHES=(
    "benchmarks/numeric_loop.py|int and float loops, recursion|10"
    "benchmarks/game_state_dicts.py|game state in dicts, 200k frames|5"
    "benchmarks/game_state_classes.py|the same state in classes|3"
    "examples/apps/gui/headless.py|300 balls, pairwise collisions|5"
    "examples/apps/ai/bench_bpe.py|BPE tokenizer training, string-heavy|1.3"
    "benchmarks/collections_bench.py|word counts, sorting, a prime sieve|1"
)

commit=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
dirty=$(git status --porcelain 2>/dev/null | grep -v '^??' | head -1)
[ -n "$dirty" ] && commit="$commit+changes"
cpu=$(grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2 | sed 's/^ *//')
[ -z "$cpu" ] && cpu=$(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo unknown)

fmt() { # milliseconds -> "626 ms" or "3.3 s"
    awk -v m="$1" 'BEGIN { if (m >= 1000) printf "%.2g s", m / 1000; else if (m >= 100) printf "%.0f ms", m; else printf "%.1f ms", m }'
}

table="| Program | CPython | pyrock | Speedup |
|---|---|---|---|"
failed=0
for b in "${BENCHES[@]}"; do
    IFS='|' read -r file what floor <<<"$b"
    dir=$(dirname "$file")
    name=$(basename "$file")
    out=$(cd "$dir" && "$bin" bench "$name" --runs "$runs" 2>&1)
    if [ $? -ne 0 ]; then
        echo "FAILED  $file" >&2
        echo "$out" | sed 's/^/    /' >&2
        failed=1
        continue
    fi
    py=$(echo "$out" | awk '/python   : median/ { print $4 }')
    nat=$(echo "$out" | awk '/native   : median/ { print $4 }')
    sp=$(echo "$out" | awk '/speedup  :/ { sub("x", "", $3); print $3 }')
    printf '%-40s %8s ms  %8s ms  %6sx\n' "$file" "$py" "$nat" "$sp" >&2
    table="$table
| \`$file\` ($what) | $(fmt "$py") | $(fmt "$nat") | $(printf '%.1f' "$sp")x |"
    if [ $gate -eq 1 ] && awk -v s="$sp" -v f="$floor" 'BEGIN { exit !(s < f) }'; then
        echo "::error title=benchmark regression::$file: ${sp}x is below its floor of ${floor}x"
        failed=1
    fi
done

echo
echo "Measured at commit $commit with \`scripts/bench_all.sh --runs $runs\` ($cpu; $(python3 --version 2>&1); $(rustc --version 2>/dev/null | cut -d' ' -f1-2)). Medians of $runs runs, wall clock including process start, \`--release\`, identical stdout."
echo
echo "$table"
exit $failed
