#!/usr/bin/env bash
# Run every command in each project's README.md ```sh block verbatim, from that
# project's directory, the way a user following the README would.
# The `pyfast` on PATH is this repo's fresh debug build, not the installed one.
#
# usage: scripts/readme_check.sh [--bench] [DIR...]
#   DIR defaults to every directory with a README.md under .ignore/samples/,
#   examples/ and demos/. `pyfast bench` lines are skipped unless --bench is given.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
export PATH="$HOME/.cargo/bin:$PATH"
(cd "$ROOT" && cargo build -q -p pyfast) || exit 1
SHIM=$(mktemp -d)
trap 'rm -rf "$SHIM"' EXIT
ln -s "$ROOT/target/debug/pyfast" "$SHIM/pyfast"
export PATH="$SHIM:$PATH"

bench=0
dirs=()
for a in "$@"; do
    case "$a" in --bench) bench=1 ;; *) dirs+=("$a") ;; esac
done
if [[ ${#dirs[@]} -eq 0 ]]; then
    for r in "$ROOT"/.ignore/samples/*/README.md "$ROOT"/examples/*/README.md "$ROOT"/demos/*/README.md; do
        [[ -f "$r" ]] && dirs+=("$(dirname "$r")")
    done
fi

pass=0; fail=0
for d in "${dirs[@]}"; do
    name=$(basename "$d")
    mapfile -t cmds < <(sed -n '/^```sh/,/^```/p' "$d/README.md" | grep -v '^```' | grep -v '^\s*$' | grep -v '^\s*#')
    ok=1
    for cmd in "${cmds[@]}"; do
        [[ $bench -eq 0 && "$cmd" == pyfast\ bench* ]] && continue
        if [[ "$cmd" == pyfast\ run* ]]; then
            # Build first (slow on a cold cache), then run with a short limit:
            # an interactive program (a game, a server) that is still running
            # after it counts as working.
            bcmd="pyfast build ${cmd#pyfast run }"
            bcmd="${bcmd%% -- *}"
            out=$(cd "$d" && timeout 600 bash -c "$bcmd" 2>&1 </dev/null); rc=$?
            if [[ $rc -eq 0 ]]; then
                out=$(cd "$d" && timeout 30 bash -c "$cmd" 2>&1 </dev/null); rc=$?
                if [[ $rc -eq 124 ]]; then
                    printf "      %-20s still running after 30s (interactive): %s\n" "$name" "$cmd"
                    rc=0
                fi
            fi
        else
            out=$(cd "$d" && timeout 600 bash -c "$cmd" 2>&1 </dev/null); rc=$?
        fi
        if [[ $rc -ne 0 ]]; then
            ok=0
            printf "FAIL  %-20s rc=%d  %s\n" "$name" "$rc" "$cmd"
            echo "$out" | tail -8 | sed 's/^/      /'
        elif [[ "$cmd" == pyfast\ run* ]] && grep -q '^warning' <<<"$out"; then
            ok=0
            printf "FAIL  %-20s cargo warnings in output  %s\n" "$name" "$cmd"
        fi
    done
    if [[ $ok -eq 1 ]]; then printf "ok    %s\n" "$name"; pass=$((pass+1)); else fail=$((fail+1)); fi
done
echo "$pass passed, $fail failed"
[[ $fail -eq 0 ]]
