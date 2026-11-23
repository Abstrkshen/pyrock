#!/usr/bin/env bash
# Put the raylib 5.5 shared library (zlib licence, official Linux x86-64 release)
# into the directory given as the first argument.
#   bash fetch-raylib.sh raylib/lib
set -euo pipefail
dest="${1:?usage: fetch-raylib.sh DEST_DIR}"
if [ -e "$dest/libraylib.so.550" ]; then
    exit 0
fi
mkdir -p "$dest"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -fsSL https://github.com/raysan5/raylib/releases/download/5.5/raylib-5.5_linux_amd64.tar.gz | tar -xz -C "$tmp"
cp -P "$tmp"/raylib-5.5_linux_amd64/lib/libraylib.so* "$dest"/
echo "raylib 5.5 -> $dest"
