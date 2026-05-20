#!/usr/bin/env bash
# Publish a stable Aperio compiler binary to bin/aperio for
# downstream app teams (fathom, pond, mdgw, etc.) to pin against.
#
# The intent: my in-flight compiler refactors churn
# target/release/aperio on every build. App teams pinning to
# bin/aperio get a snapshot that only moves when this script runs.
#
# IMPORTANT: a publish is a SIDE-EFFECT that affects running app
# sessions pinned to bin/aperio. Never publish without first
# running a meaningful regression suite — at minimum the full
# workspace test on the commit being published. The
# `--validated` flag is the explicit gate; running this script
# without it refuses to publish and prints what the operator
# should do first.
#
# Usage:
#   scripts/publish-stable.sh --validated [--skip-build]
#
# Flags:
#   --validated     Required. Confirms regression suite passed on
#                   the commit being published. Operator's
#                   responsibility — this script does NOT run the
#                   tests itself.
#   --skip-build    Reuse target/release/aperio instead of
#                   re-running `cargo build --release`. Useful
#                   when the binary was built earlier in the
#                   validation step.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VALIDATED=0
SKIP_BUILD=0
for arg in "$@"; do
    case "$arg" in
        --validated)  VALIDATED=1 ;;
        --skip-build) SKIP_BUILD=1 ;;
        -h|--help)
            sed -n '1,30p' "$0"
            exit 0
            ;;
        *)
            echo "error: unknown flag $arg" >&2
            echo "usage: scripts/publish-stable.sh --validated [--skip-build]" >&2
            exit 2
            ;;
    esac
done

if [[ "$VALIDATED" -eq 0 ]]; then
    cat >&2 <<'MSG'
error: refusing to publish without --validated.

A publish to bin/aperio affects every app session pinned to it.
Before passing --validated, you must have:

  1. Committed the change(s) you want to publish.
  2. Run the regression suite on that commit:
       cargo test --release --workspace -- --test-threads=1
     (or, at minimum, your targeted smoke set covering the
     primitives the change touches).
  3. Confirmed all tests pass.

If the suite passes, re-run with:
  scripts/publish-stable.sh --validated [--skip-build]
MSG
    exit 1
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

# Refuse to publish from a dirty tree — the VERSION marker would
# be ambiguous about what's actually in the binary. Operator
# must commit (or stash) first.
if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "error: refusing to publish from a dirty working tree." >&2
    echo "Commit or stash your changes first; the bin/VERSION marker" >&2
    echo "must point at a real commit, not an uncommitted snapshot." >&2
    exit 1
fi

mkdir -p "$ROOT/bin"
cp "$SRC" "$DST"
chmod +x "$DST"

GIT_SHA="$(git rev-parse HEAD)"
GIT_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

cat > "$ROOT/bin/VERSION" <<EOF
aperio published-stable snapshot
commit: $GIT_SHA
branch: $GIT_BRANCH
date:   $DATE
EOF

echo "==> published $DST"
echo "    commit:  $GIT_SHA"
echo "    branch:  $GIT_BRANCH"
echo "    date:    $DATE"
