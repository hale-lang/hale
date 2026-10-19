#!/usr/bin/env bash
# Take down every compose project whose working directory is <root> or
# under it — its containers, networks and volumes — as dna::scratch_done
# does for a fixture's root. A `hale` verb with no
# HALE_DNA_MEMORY_DSN_OWNER brings the organism's memory up as an
# allocation of the local habitat, which a script keeps under its root
# (XDG_DATA_HOME there), and nothing else takes it down when a script's
# temporary root goes.
#
# GH #1653: the teardown is `hale habitat sweep <root>`, the one `hale
# test` and the CLI tests' reap share. A project is found by the
# com.docker.compose.project.working_dir label on any container, running
# or not, or by name in <root>/.dna-compose-projects, where the sweep names
# every project before it takes any down and drops the name only once the
# project's containers and volumes are confirmed gone, so a teardown that
# did not finish is retried by the next. It never touches the developer's
# own habitat (under $HOME), nor the one XDG_DATA_HOME names outside
# <root>. Exit 0 when nothing is left, or there is no docker to ask; exit 1
# naming what failed or survived — the caller keeps <root> too.
# HALE_DNA_KEEP_SCRATCH=1 keeps everything. The toolchain is $HALE_BIN, or
# `hale` on PATH.
#
#   scripts/compose-down-under.sh <root>
set -uo pipefail
root=${1:?the root whose compose projects go}
root=${root%/}
case "${HALE_DNA_KEEP_SCRATCH:-}" in ''|0) ;; *) exit 0 ;; esac
command -v docker >/dev/null 2>&1 || exit 0
if ! "${HALE_BIN:-hale}" habitat sweep "$root"; then
    printf 'compose-down-under: %s is kept, naming its projects in %s, for another attempt\n' "$root" "$root/.dna-compose-projects" >&2
    exit 1
fi
exit 0
