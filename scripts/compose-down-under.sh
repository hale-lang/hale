#!/usr/bin/env bash
# Take down every compose project whose working directory is <root> or
# under it — its containers, networks and volumes — as dna::scratch_done
# does for a fixture's root. A `hale` verb with no
# HALE_DNA_MEMORY_DSN_OWNER brings the organism's memory up as an
# allocation of the local habitat, which a script keeps under its root
# (XDG_DATA_HOME there), and nothing else takes it down when a script's
# temporary root goes. Never the developer's own habitat (under $HOME),
# nor the one XDG_DATA_HOME names outside <root>: the machine's habitat is
# long-lived, and no teardown's (GH #1653).
#
# A project is found by the com.docker.compose.project.working_dir label on
# any container, running or not, or by name in <root>/.dna-compose-projects,
# where a teardown that did not finish left it (a volume carries its
# project's name but not the directory it ran from). `down` by project name
# needs no compose file, and the teardown is confirmed by the project's
# label on containers and on volumes. Exit 0 when nothing is left, or there
# is no docker to ask (no CLI, or no daemon); exit 1 naming what failed or
# survived, with the projects kept named under <root> for another attempt —
# the caller keeps <root> too. HALE_DNA_KEEP_SCRATCH=1 keeps everything.
#
#   scripts/compose-down-under.sh <root>
set -uo pipefail
root=${1:?the root whose compose projects go}
root=${root%/}
file=$root/.dna-compose-projects
case "${HALE_DNA_KEEP_SCRATCH:-}" in ''|0) ;; *) exit 0 ;; esac
command -v docker >/dev/null 2>&1 || exit 0

# docker's answer on stdout; status 2 when there is no daemon to ask
ask() {
    local out err
    err=$(mktemp) || return 1
    if out=$(docker "$@" 2>"$err"); then rm -f -- "$err"; printf '%s' "$out"; return 0; fi
    if grep -q -e 'Cannot connect to the Docker daemon' -e 'Is the docker daemon running' "$err"; then rm -f -- "$err"; return 2; fi
    printf 'compose-down-under: docker %s: %s\n' "$*" "$(cat -- "$err")" >&2
    rm -f -- "$err"
    return 1
}

# the machine's habitats this teardown never touches
habitat_project() { printf 'hale-habitat-local-%s' "$(printf '%s' "$1" | sha256sum | cut -c1-8)"; }
mine=$(habitat_project "$HOME/.local/share/hale/habitat/local")
here=${XDG_DATA_HOME:-$HOME/.local/share}/hale/habitat/local
current=
if [[ "$here" != "$root" && "$here" != "$root"/* ]]; then current=$(habitat_project "$here"); fi

listed=$(ask ps -a --filter label=com.docker.compose.project \
    --format '{{.Label "com.docker.compose.project"}}|{{.Label "com.docker.compose.project.working_dir"}}')
case $? in 0) ;; 2) exit 0 ;; *) exit 1 ;; esac
projects=$(
    {
        printf '%s\n' "$listed" | while IFS='|' read -r project wd; do
            [[ -n "$project" ]] || continue
            if [[ "$wd" == "$root" || "$wd" == "$root"/* ]]; then printf '%s\n' "$project"; fi
        done
        [[ -f "$file" ]] && cat -- "$file"
    } | sed '/^$/d' | grep -v -x -F -e "$mine" ${current:+-e "$current"} | sort -u
)
[[ -n "$projects" ]] || exit 0
mkdir -p -- "$root" && printf '%s\n' "$projects" >"$file"
failed=0
while read -r project; do
    ask compose -p "$project" down -v --remove-orphans >/dev/null; [[ $? -eq 1 ]] && failed=1
    for what in "ps -a -q" "volume ls -q"; do
        # shellcheck disable=SC2086
        left=$(ask $what --filter "label=com.docker.compose.project=$project"); rc=$?
        if [[ $rc -eq 1 ]]; then failed=1
        elif [[ -n "$left" ]]; then
            kind=containers; [[ "$what" == volume* ]] && kind=volumes
            printf 'compose-down-under: %s still has %s after its teardown\n' "$project" "$kind" >&2; failed=1
        fi
    done
done <<<"$projects"
if ((failed)); then
    printf 'compose-down-under: %s is kept, naming its projects in %s, for another attempt\n' "$root" "$file" >&2
    exit 1
fi
rm -f -- "$file"
exit 0
