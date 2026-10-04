#!/usr/bin/env bash
# Fails if the normal dependency tree of soroban-failure-analysis contains a
# crate that is not on the allowlist.
#
# The analysis crate must perform no I/O (docs/architecture/purity.md). A
# dependency that can reach the network, filesystem or clock would break that
# silently, so this check makes the boundary mechanical rather than a matter of
# review. The allowlist lives in .github/analysis-dependency-allowlist.txt.
#
# Usage: scripts/check-analysis-purity.sh   (run from the repository root)

set -euo pipefail

ALLOWLIST=".github/analysis-dependency-allowlist.txt"
CRATE="soroban-failure-analysis"

if [ ! -f "$ALLOWLIST" ]; then
    echo "error: allowlist not found at $ALLOWLIST" >&2
    exit 2
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Resolved crate names only (the version and path columns are dropped).
cargo tree -p "$CRATE" --edges normal --prefix none --format "{p}" \
    | awk '{print $1}' | sort -u > "$tmp/actual"

# Allowlist: strip comments and blank lines.
grep -vE '^\s*(#|$)' "$ALLOWLIST" | awk '{print $1}' | sort -u > "$tmp/allowed"

comm -23 "$tmp/actual" "$tmp/allowed" > "$tmp/disallowed"

if [ -s "$tmp/disallowed" ]; then
    echo "error: $CRATE depends on crates that are not on the purity allowlist:" >&2
    sed 's/^/  - /' "$tmp/disallowed" >&2
    echo >&2
    echo "This crate must perform no I/O. See docs/architecture/purity.md." >&2
    echo "If a crate is genuinely pure, add it to $ALLOWLIST and justify it in the PR." >&2
    exit 1
fi

count=$(wc -l < "$tmp/actual" | tr -d ' ')
echo "ok: all $count crates in the $CRATE normal dependency tree are on the allowlist."
