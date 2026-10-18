#!/usr/bin/env bash
# Take down every compose project whose working directory is <root> or
# under it — its containers, networks and volumes — as dna::scratch_done
# does for a fixture's root. A `hale dna` verb with no
# HALE_DNA_MEMORY_DSN_OWNER brings the knowledge database up from the
# project's dna/compose.yaml, and nothing else takes it down when a
# script's temporary root goes. Projects are found by the
# com.docker.compose.project.working_dir label on any container, running
# or not; `down` by project name needs no compose file.
# HALE_DNA_KEEP_SCRATCH=1 keeps them. Without docker nothing is done.
#
#   scripts/compose-down-under.sh <root>
set -uo pipefail
root=${1:?the root whose compose projects go}
root=${root%/}
case "${HALE_DNA_KEEP_SCRATCH:-}" in ''|0) ;; *) exit 0 ;; esac
command -v docker >/dev/null 2>&1 || exit 0
docker ps -a --filter label=com.docker.compose.project \
    --format '{{.Label "com.docker.compose.project"}}|{{.Label "com.docker.compose.project.working_dir"}}' 2>/dev/null |
while IFS='|' read -r project wd; do
    [[ -n "$project" ]] || continue
    if [[ "$wd" == "$root" || "$wd" == "$root"/* ]]; then echo "$project"; fi
done | sort -u | while read -r project; do
    docker compose -p "$project" down -v --remove-orphans >/dev/null 2>&1 ||
        echo "compose-down-under: $project under $root did not come down" >&2
done
exit 0
