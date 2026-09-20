#!/usr/bin/env bash
set -euo pipefail
: "${HALE_HOST_LAUNCH_EVIDENCE:?explicit evidence directory required}"
mkdir -p "$HALE_HOST_LAUNCH_EVIDENCE"
launch_test_here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# The runtime sweeps stale observation SHM. Share a private PID/IPC/mount
# namespace for actual host, children, reader and observer; no host fallback.
export HALE_HOST_LAUNCH_NAMESPACE=private
exec /usr/bin/bwrap --unshare-user --unshare-ipc --unshare-pid --ro-bind / / \
  --dev /dev --tmpfs /dev/shm --tmpfs /tmp \
  --bind "$HALE_HOST_LAUNCH_EVIDENCE" "$HALE_HOST_LAUNCH_EVIDENCE" \
  --proc /proc --die-with-parent -- node "$launch_test_here/acceptance.mjs" "$@"
