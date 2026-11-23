#!/usr/bin/env bash
# Prepare site/ for publishing: the playground's compiler (crates/playground built
# for WebAssembly) and its example programs (examples/playground). Both are build
# outputs, ignored by git. .github/workflows/pages.yml runs this before deploying;
# run it locally, then serve site/ (python3 -m http.server -d site) to try the page.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
cargo build -q --release --locked -p pyrock-playground --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/pyrock_playground.wasm site/pyrock.wasm
rm -rf site/examples
mkdir -p site/examples
cp examples/playground/*.py site/examples/
echo "site/pyrock.wasm ($(wc -c < site/pyrock.wasm) bytes), site/examples/ ($(ls site/examples | wc -l) programs)"
