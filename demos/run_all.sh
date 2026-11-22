#!/usr/bin/env bash
# Build and run each demo, compare stdout with its expected.stdout.
# usage: demos/run_all.sh
set -u
cd "$(dirname "$0")"
ROOT=$(cd .. && pwd)
export PATH="$HOME/.cargo/bin:$PATH"
(cd "$ROOT" && cargo build -q -p pyfast) || exit 1
PYFAST="$ROOT/target/debug/pyfast"
# demo directory, then the program and its arguments
CASES=(
    "backend main.py"
    "ai main.py"
    "gui headless.py -- 300"
)
pass=0; fail=0
for c in "${CASES[@]}"; do
    read -r dir args <<<"$c"
    got=$(cd "$dir" && $PYFAST run $args 2>/tmp/pyfast_demo_err.$$); rc=$?
    if [[ $rc -eq 0 && "$got" == "$(cat "$dir/expected.stdout")" ]]; then
        printf "ok    %s\n" "$dir"; pass=$((pass+1))
    else
        printf "FAIL  %s (exit %d)\n" "$dir" "$rc"
        diff <(echo "$got") "$dir/expected.stdout" | head -20 | sed 's/^/      /'
        tail -5 /tmp/pyfast_demo_err.$$ | sed 's/^/      /'
        fail=$((fail+1))
    fi
done
rm -f /tmp/pyfast_demo_err.$$
echo "$pass passed, $fail failed"
[[ $fail -eq 0 ]]
