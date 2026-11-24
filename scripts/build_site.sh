#!/usr/bin/env bash
# Prepare site/ for publishing. All of it is build output, ignored by git:
#
# * site/pyrock.wasm: the compiler (crates/playground) as WebAssembly, for Compile.
# * site/examples/*.py: the playground's programs (examples/playground).
# * site/examples/*.wasm: each program compiled by pyrock (`pyrock build --release
#   --mode native-only --target wasm32-wasip1`), for Run. Each one is run here with
#   Node and the page's own WASI shim, and compared with CPython's output.
# * site/examples/compiled.json: per program, its .wasm and the result of that
#   comparison, or why it has no browser build.
#
# .github/workflows/pages.yml runs this before deploying; run it locally, then serve
# site/ (python3 -m http.server -d site) to try the page.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"
rustup target add wasm32-unknown-unknown wasm32-wasip1 >/dev/null 2>&1 || true

cargo build -q --release --locked -p pyrock-playground --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/pyrock_playground.wasm site/pyrock.wasm
cargo build -q --release --locked -p pyrock
PYROCK="$ROOT/target/release/pyrock"

rm -rf site/examples
mkdir -p site/examples
cp examples/playground/*.py site/examples/

# Known differences between a program's browser build and CPython, with the reason
# shown on the page. Anything else that differs fails the build.
declare -A KNOWN=(
    [svg_double_pendulum]="The pendulums are chaotic: WebAssembly's sin and cos round the last digit differently from CPython's math library, and the difference grows until a few late frames print one coordinate differently. The native executable matches CPython exactly."
)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
manifest="{"
sep=""
failed=0
for src in examples/playground/*.py; do
    name=$(basename "$src" .py)
    entry=""
    if wasm=$("$PYROCK" build "$src" --release --mode native-only --target wasm32-wasip1 --out "$work/$name" 2>"$work/$name.build"); then
        python3 "$src" >"$work/$name.py.out" 2>/dev/null && py=0 || py=$?
        node scripts/wasi_run.cjs "$wasm" >"$work/$name.wasm.out" 2>"$work/$name.wasm.err" && wc=0 || wc=$?
        if [ "$py" = "$wc" ] && cmp -s "$work/$name.py.out" "$work/$name.wasm.out"; then
            cp "$wasm" "site/examples/$name.wasm"
            entry="{\"wasm\": \"$name.wasm\", \"bytes\": $(wc -c <"$wasm")}"
        elif grep -q "generators are not supported in pyrock's WebAssembly build" "$work/$name.wasm.err"; then
            entry="{\"why\": \"It uses a generator. Generators need stack switching, which WebAssembly doesn't have, so pyrock's browser build can't run it (the native executable can).\"}"
        elif [ -n "${KNOWN[$name]:-}" ]; then
            cp "$wasm" "site/examples/$name.wasm"
            note=$(printf '%s' "${KNOWN[$name]}" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')
            entry="{\"wasm\": \"$name.wasm\", \"bytes\": $(wc -c <"$wasm"), \"note\": $note}"
        else
            echo "FAILED  $name: the WebAssembly build's output differs from CPython's" >&2
            diff <(head -c 2000 "$work/$name.py.out") <(head -c 2000 "$work/$name.wasm.out") | head -5 >&2 || true
            tail -3 "$work/$name.wasm.err" >&2
            failed=1
            continue
        fi
    elif grep -q "P2[0-9][0-9]  error" "$work/$name.build"; then
        entry="{\"why\": \"It uses a CPython module, and a WebAssembly build can't embed CPython. On your machine pyrock compiles it in hybrid mode, with CPython at the boundaries Explain lists.\"}"
    else
        entry="{\"why\": \"pyrock rejects it at compile time: see the Check tab.\"}"
    fi
    manifest="$manifest$sep\"$name.py\": $entry"
    sep=", "
done
echo "$manifest}" | python3 -m json.tool >site/examples/compiled.json
[ $failed -eq 0 ] || exit 1
echo "site/pyrock.wasm ($(wc -c <site/pyrock.wasm) bytes), site/examples/: $(ls site/examples/*.py | wc -l) programs, $(ls site/examples/*.wasm | wc -l) compiled to WebAssembly"
