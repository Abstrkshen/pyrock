#!/bin/sh
# Install a prebuilt pyrock binary from the GitHub releases.
#
#   curl -fsSL https://raw.githubusercontent.com/Abstrkshen/pyrock/main/install.sh | sh
#
# Environment variables:
#   PYROCK_VERSION      a release tag such as v0.1.0 (default: the latest release)
#   PYROCK_INSTALL_DIR  where to put the binary (default: ~/.local/bin)
set -eu

repo=Abstrkshen/pyrock
version=${PYROCK_VERSION:-latest}
bin_dir=${PYROCK_INSTALL_DIR:-$HOME/.local/bin}

say() { printf 'pyrock: %s\n' "$1"; }
die() { printf 'pyrock: %s\n' "$1" >&2; exit 1; }

os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
    Linux/x86_64 | Linux/amd64) target=x86_64-unknown-linux-musl ;;
    Linux/aarch64 | Linux/arm64) target=aarch64-unknown-linux-musl ;;
    Darwin/arm64) target=aarch64-apple-darwin ;;
    *) die "no prebuilt binary for $os $arch. Build from source instead:
    cargo install --git https://github.com/$repo pyrock" ;;
esac

if [ "$version" = latest ]; then
    base=https://github.com/$repo/releases/latest/download
else
    base=https://github.com/$repo/releases/download/$version
fi

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$1" -O "$2"
    else
        die "needs curl or wget"
    fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

archive=pyrock-$target.tar.gz
say "downloading $base/$archive"
fetch "$base/$archive" "$tmp/$archive" || die "download failed: $base/$archive"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || die "download failed: $base/SHA256SUMS"

expected=$(grep " $archive\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
[ -n "$expected" ] || die "$archive is not listed in SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp/$archive" | cut -d' ' -f1)
else
    actual=$(shasum -a 256 "$tmp/$archive" | cut -d' ' -f1)
fi
[ "$actual" = "$expected" ] || die "checksum mismatch for $archive"

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$bin_dir"
cp "$tmp/pyrock-$target/pyrock" "$bin_dir/pyrock.tmp"
chmod 755 "$bin_dir/pyrock.tmp"
mv "$bin_dir/pyrock.tmp" "$bin_dir/pyrock"
say "installed $("$bin_dir/pyrock" --version) to $bin_dir/pyrock"

case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) say "add $bin_dir to your PATH, for example: export PATH=\"$bin_dir:\$PATH\"" ;;
esac
if ! command -v cargo >/dev/null 2>&1 && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    say "pyrock builds programs with Cargo; install Rust from https://rustup.rs"
fi
if ! command -v python3 >/dev/null 2>&1; then
    say "pyrock parses Python with python3; install Python 3"
fi
