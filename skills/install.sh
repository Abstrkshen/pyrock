#!/usr/bin/env bash
# Install the pyrock skill globally for Claude Code, Codex and Cursor agents.
#
#   bash skills/install.sh           # symlink (edits in this repo are live everywhere)
#   bash skills/install.sh --copy    # copy instead, for a tool that doesn't follow symlinks
#   bash skills/install.sh --remove  # uninstall from all three
#
# Targets (each tool's personal/global skills directory):
#   Claude Code  ${CLAUDE_SKILLS_DIR:-~/.claude/skills}/pyrock
#   Codex        ${CODEX_HOME:-~/.codex}/skills/pyrock
#   Cursor       ~/.cursor/skills/pyrock   (not ~/.cursor/skills-cursor, which Cursor owns)
set -euo pipefail

mode="link"
case "${1:-}" in
    --copy) mode="copy" ;;
    --remove) mode="remove" ;;
    "") ;;
    *) echo "usage: $0 [--copy|--remove]" >&2; exit 2 ;;
esac

src="$(cd "$(dirname "$0")" && pwd)/pyrock"
targets=(
    "Claude Code|${CLAUDE_SKILLS_DIR:-$HOME/.claude/skills}/pyrock"
    "Codex|${CODEX_HOME:-$HOME/.codex}/skills/pyrock"
    "Cursor|$HOME/.cursor/skills/pyrock"
)

for entry in "${targets[@]}"; do
    tool="${entry%%|*}"
    dest="${entry#*|}"
    # Only replace what this script created: a symlink, or a copy marked with .pyrock-skill.
    if [ -e "$dest" ] && [ ! -L "$dest" ] && [ ! -f "$dest/.pyrock-skill" ]; then
        echo "skip $tool: $dest exists and was not installed by this script" >&2
        continue
    fi
    rm -rf "$dest"
    case "$mode" in
        remove)
            echo "removed  $tool: $dest"
            ;;
        link)
            mkdir -p "$(dirname "$dest")"
            ln -s "$src" "$dest"
            echo "linked   $tool: $dest -> $src"
            ;;
        copy)
            mkdir -p "$(dirname "$dest")"
            cp -R "$src" "$dest"
            touch "$dest/.pyrock-skill"
            echo "copied   $tool: $dest"
            ;;
    esac
done
