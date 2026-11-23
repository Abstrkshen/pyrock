#!/usr/bin/env bash
# Run every command in each project's README.md ```sh block verbatim, from that
# project's directory, the way a user following the README would.
# The `pyrock` on PATH is this repo's fresh debug build, not the installed one.
#
# usage: scripts/readme_check.sh [--bench] [DIR...]
#   DIR defaults to every directory with a README.md under examples/ and demos/.
#   `pyrock bench` lines are skipped unless --bench is given. With CI set (as in
#   GitHub Actions), a README containing `<!-- readme_check: skip-ci ... -->` is
#   skipped: it needs something CI doesn't have (a camera, Ollama, a venv).
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
export PATH="$HOME/.cargo/bin:$PATH"
(cd "$ROOT" && cargo build -q -p pyrock) || exit 1
SHIM=$(mktemp -d)
trap 'rm -rf "$SHIM"' EXIT
ln -s "$ROOT/target/debug/pyrock" "$SHIM/pyrock"
export PATH="$SHIM:$PATH"

bench=0
dirs=()
for a in "$@"; do
    case "$a" in --bench) bench=1 ;; *) dirs+=("$a") ;; esac
done
if [[ ${#dirs[@]} -eq 0 ]]; then
    for r in "$ROOT"/examples/*/README.md "$ROOT"/demos/*/README.md; do
        [[ -f "$r" ]] && dirs+=("$(dirname "$r")")
    done
fi

pass=0; fail=0
for d in "${dirs[@]}"; do
    name=$(basename "$d")
    if [[ -n "${CI:-}" ]] && grep -q "readme_check: skip-ci" "$d/README.md"; then
        printf "skip  %s %s\n" "$name" "$(grep -o 'skip-ci[^>]*' "$d/README.md" | sed 's/skip-ci *//; s/ *--$//')"
        continue
    fi
    mapfile -t cmds < <(sed -n '/^```sh/,/^```/p' "$d/README.md" | grep -v '^```' | grep -v '^\s*$' | grep -v '^\s*#')
    ok=1
    # A block that starts a background job (`cmd &` ... `kill %1`) is one shell
    # session: run it as a single script, in its own process group, and pass if
    # the session reaches its end successfully.
    if printf '%s\n' "${cmds[@]}" | grep -qE '&\s*$|kill %'; then
        script=$(printf '%s\n' "${cmds[@]}" | grep -v "^pyrock bench" )
        [[ $bench -eq 1 ]] && script=$(printf '%s\n' "${cmds[@]}")
        # Give a background server time to come up before the next command.
        script=$(sed -E 's/&\s*$/\& sleep 5/' <<<"$script")
        out=$(cd "$d" && timeout 900 bash -c "set -m; bash -c \"\$0\" & p=\$!; trap 'kill -TERM -- -\$p 2>/dev/null; exit 124' TERM; wait \$p" "$script" 2>&1 </dev/null); rc=$?
        if [[ $rc -ne 0 ]]; then
            printf "FAIL  %-20s rc=%d  (shell session)\n" "$name" "$rc"
            echo "$out" | tail -8 | sed 's/^/      /'
            fail=$((fail+1))
        else
            printf "ok    %s (shell session)\n" "$name"; pass=$((pass+1))
        fi
        continue
    fi
    for cmd in "${cmds[@]}"; do
        [[ $bench -eq 0 && "$cmd" == pyrock\ bench* ]] && continue
        if [[ "$cmd" == pyrock\ run* ]]; then
            # Build first (slow on a cold cache), then run with a short limit:
            # an interactive program (a game, a server) that is still running
            # after it counts as working.
            bcmd="pyrock build ${cmd#pyrock run }"
            bcmd="${bcmd%% -- *}"
            out=$(cd "$d" && timeout 600 bash -c "$bcmd" 2>&1 </dev/null); rc=$?
            if [[ $rc -eq 0 ]]; then
                # `set -m` puts the command in its own process group, and the
                # trap kills that whole group on timeout: a server started by
                # `pyrock run` must not outlive the check (it would keep this
                # command substitution's pipe open forever).
                out=$(cd "$d" && timeout 30 bash -c "set -m; $cmd & p=\$!; trap 'kill -TERM -- -\$p 2>/dev/null; exit 124' TERM; wait \$p" 2>&1 </dev/null); rc=$?
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
        elif [[ "$cmd" == pyrock\ run* ]] && grep -q '^warning' <<<"$out"; then
            ok=0
            printf "FAIL  %-20s cargo warnings in output  %s\n" "$name" "$cmd"
        fi
    done
    if [[ $ok -eq 1 ]]; then printf "ok    %s\n" "$name"; pass=$((pass+1)); else fail=$((fail+1)); fi
done
echo "$pass passed, $fail failed"
[[ $fail -eq 0 ]]
