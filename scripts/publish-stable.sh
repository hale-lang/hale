#!/usr/bin/env bash
# Publish a stable Aperio compiler binary to bin/aperio for
# downstream app teams (fathom, pond, mdgw, etc.) to pin against.
#
# The intent: my in-flight compiler refactors churn
# target/release/aperio on every build. App teams pinning to
# bin/aperio get a snapshot that only moves when this script runs.
#
# Usage:   scripts/publish-stable.sh
# Optional: scripts/publish-stable.sh --skip-build   (uses whatever's
#                                                    already at
#                                                    target/release/aperio)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SKIP_BUILD=0
if [[ "${1:-}" == "--skip-build" ]]; then
    SKIP_BUILD=1
fi

if [[ "$SKIP_BUILD" -eq 0 ]]; then
    echo "==> cargo build --release"
    cargo build --release
fi

SRC="$ROOT/target/release/aperio"
DST="$ROOT/bin/aperio"

if [[ ! -x "$SRC" ]]; then
    echo "error: $SRC missing or not executable" >&2
    exit 1
fi

mkdir -p "$ROOT/bin"
cp "$SRC" "$DST"
chmod +x "$DST"

# Record provenance — what commit is this snapshot from, and is the
# working tree dirty when we published? App teams reading the VERSION
# file can sanity-check what they're running against.
GIT_SHA="$(git rev-parse HEAD)"
GIT_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
DIRTY=""
if ! git diff --quiet || ! git diff --cached --quiet; then
    DIRTY=" (dirty)"
fi
DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

cat > "$ROOT/bin/VERSION" <<EOF
aperio published-stable snapshot
commit: $GIT_SHA$DIRTY
branch: $GIT_BRANCH
date:   $DATE
EOF

echo "==> published $DST"
echo "    commit:  $GIT_SHA$DIRTY"
echo "    branch:  $GIT_BRANCH"
echo "    date:    $DATE"
