#!/usr/bin/env bash
# Build lib/liblumen.so (the C layer over raylib) and fetch raylib next to it.
# Needs a C compiler; the game also needs librsvg and cairo at runtime
# (Debian/Ubuntu: sudo apt install librsvg2-2 libcairo2).
set -euo pipefail
cd "$(dirname "$0")"
bash ../fetch-raylib.sh lib
cc -shared -fPIC -O2 -I../raylib -o lib/liblumen.so rl.c -ldl
echo "built lib/liblumen.so"
