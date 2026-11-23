#!/usr/bin/env bash
# Build every tour program with pyrock, run it, and compare with CPython.
#   - NN_name.py           : compared with `python3 NN_name.py`
#   - NN_name/main.py      : compared with CPython, or with expected.txt /
#                            expected.err when it imports Rust or C
# Compared: stdout, exit status and the last stderr line (the exception).
#
# usage: examples/tour/run_all.sh [--release] [name-filter]
set -u
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
PYROCK="$ROOT/target/debug/pyrock"
PROFILE=()
FILTER=""
for a in "$@"; do
    case "$a" in
        --release) PROFILE=(--release) ;;
        *) FILTER="$a" ;;
    esac
done
export PATH="$HOME/.cargo/bin:$PATH"
export PYROCK_TARGET_DIR="$PWD/.pyrock/target"
(cd "$ROOT" && cargo build -q -p pyrock) || exit 1

pass=0; fail=0
for sample in [0-9][0-9]_*; do
    [[ -n "$FILTER" && "$sample" != *"$FILTER"* ]] && continue
    if [[ -d "$sample" ]]; then main="$sample/main.py"; else main="$sample"; fi
    name="${sample%.py}"
    exe=$("$PYROCK" build "$main" "${PROFILE[@]}" --out ".pyrock/$name" 2>/tmp/pyrock_build_err.$$) || {
        printf "FAIL  %-22s build failed\n" "$name"; sed 's/^/      /' /tmp/pyrock_build_err.$$; fail=$((fail+1)); continue; }
    got_out=$("$exe" 2>/tmp/pyrock_err.$$); got_code=$?
    got_err=$(tail -n 1 /tmp/pyrock_err.$$)
    if [[ -f "$sample/expected.txt" ]]; then
        want_out=$(cat "$sample/expected.txt")
        want_err=$(cat "$sample/expected.err" 2>/dev/null || true)
        want_code=$([[ -n "$want_err" ]] && echo 1 || echo 0)
        ref="expected.txt"
    else
        want_out=$(cd "$(dirname "$main")" && python3 "$(basename "$main")" 2>/tmp/pyrock_err.$$); want_code=$?
        want_err=$(tail -n 1 /tmp/pyrock_err.$$)
        ref="CPython"
    fi
    if [[ "$got_out" == "$want_out" && "$got_code" == "$want_code" && "$got_err" == "$want_err" ]]; then
        printf "ok    %-22s matches %s\n" "$name" "$ref"; pass=$((pass+1))
    else
        printf "FAIL  %-22s differs from %s\n" "$name" "$ref"; fail=$((fail+1))
        diff <(echo "$want_out"; echo "exit=$want_code $want_err") <(echo "$got_out"; echo "exit=$got_code $got_err") | head -20 | sed 's/^/      /'
    fi
done
rm -f /tmp/pyrock_err.$$ /tmp/pyrock_build_err.$$
echo "$pass passed, $fail failed"
[[ $fail -eq 0 ]]
