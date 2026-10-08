#!/usr/bin/env bash
# Regenerate the clients committed in this repository (GH #1417, R8a), or
# with --check refuse when one no longer matches the surface it was
# generated from. A committed client names its surface's digest, and
# `hale api client --check` names the drift.
#
#   scripts/api-clients.sh           rewrite every committed client
#   scripts/api-clients.sh --check   write nothing; exit 1 on any drift
#
# `crates/hale-cli/tests/api_clients_current.rs` runs the check in CI.
set -euo pipefail
cd "$(dirname "$0")/.."
HALE=${HALE_BIN:-./target/release/hale}
[ -x "$HALE" ] || { echo "no hale binary at $HALE (cargo build --release)" >&2; exit 1; }

mode=--out
case "${1:-}" in
  "") ;;
  --check) mode=--check ;;
  *) echo "usage: scripts/api-clients.sh [--check]" >&2; exit 2 ;;
esac

# surface | target | language | file
clients=(
  "Public|tests/api-contract/program.hl|hale|tests/hale/api/client/public/client.hl"
  "Admin|tests/api-contract/program.hl|hale|tests/hale/api/client/admin/client.hl"
  "HeadCommands|dna/api|hale|dna/core/legs/head_commands/client.hl"
)

status=0
for row in "${clients[@]}"; do
  IFS='|' read -r surface target lang file <<<"$row"
  mkdir -p "$(dirname "$file")"
  HALE_SKIP_STALE_CHECK=1 "$HALE" api client --surface "$surface" --lang "$lang" "$mode" "$file" "$target" || status=1
done
exit $status
