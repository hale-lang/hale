#!/usr/bin/env bash
# The nightly build's steps (.github/workflows/nightly.yml): what a run
# builds, the version it reports, its release notes, and the pruning of old
# nightlies. Kept here, not inline in the workflow, so each step runs by
# hand against the real repository:
#
#   scripts/nightly.sh version 2026-11-21 1 <sha>   # 0.37.1-nightly.20261121+abc1234
#   scripts/nightly.sh set-version <version>        # into Cargo.toml
#   scripts/nightly.sh plan                         # in CI: GITHUB_* + GH_TOKEN
#   scripts/nightly.sh notes <tag> <version> <sha>  # the release body, on stdout
#   scripts/nightly.sh prune [--dry-run]            # keep the newest 14
#
# A nightly is tagged `nightly-YYYY-MM-DD` (a second run on one day,
# `nightly-YYYY-MM-DD.2`, and so on: a tag is never moved, so a pin on one
# always means the same build), published as a GitHub pre-release, and its
# version is `<next>-nightly.<YYYYMMDD>[.<n>]+<commit>`, where `<next>` is
# Cargo.toml's version with the patch bumped: a nightly sorts after the
# release it follows and before the next one. Only `nightly-*` names are
# ever pruned; a `v*` release or tag is never touched.
set -euo pipefail

KEEP=14
NIGHTLY_RE='^nightly-[0-9]{4}-[0-9]{2}-[0-9]{2}(\.[0-9]+)?$'
root="$(cd "$(dirname "$0")/.." && pwd)"

die() { printf 'nightly.sh: %s\n' "$*" >&2; exit 1; }
repo() { printf '%s' "${GITHUB_REPOSITORY:-hale-lang/hale}"; }

# Cargo.toml's [workspace.package] version.
cargo_version() {
  awk '/^\[/{s=($0=="[workspace.package]")} s && /^version *=/{gsub(/.*= *"|".*/,""); print; exit}' "$root/Cargo.toml"
}

# version <YYYY-MM-DD> <n> <sha>
version() {
  local day=$1 n=$2 sha=$3 base next maj min pat
  base="$(cargo_version)"
  [ -n "$base" ] || die "no [workspace.package] version in Cargo.toml"
  if [[ "$base" == *-* || "$base" == *+* ]]; then
    # a pre-release in Cargo.toml (0.38.0-rc.1) already sorts before its
    # release: the nightly is of that release
    next="${base%%[-+]*}"
  else
    IFS=. read -r maj min pat <<<"$base"
    [[ "$maj" =~ ^[0-9]+$ && "$min" =~ ^[0-9]+$ && "$pat" =~ ^[0-9]+$ ]] || die "Cargo.toml's version is not MAJOR.MINOR.PATCH: $base"
    next="$maj.$min.$((pat + 1))"
  fi
  local suffix=""
  [ "$n" -gt 1 ] && suffix=".$n"
  printf '%s-nightly.%s%s+%s\n' "$next" "${day//-/}" "$suffix" "${sha:0:7}"
}

# set-version <version>: rewrite [workspace.package] version in Cargo.toml,
# which every crate inherits (cargo updates Cargo.lock's entries for them).
set_version() {
  local v=$1 tmp
  [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]] || die "not a semver version: $v"
  tmp="$(mktemp)"
  awk -v v="$v" '/^\[/{s=($0=="[workspace.package]")} s && !done && /^version *=/{print "version = \"" v "\""; done=1; next} {print}' "$root/Cargo.toml" >"$tmp"
  cat "$tmp" >"$root/Cargo.toml"
  rm -f "$tmp"
  [ "$(cargo_version)" = "$v" ] || die "Cargo.toml's version did not become $v"
  echo "Cargo.toml: version = \"$v\""
}

# The repository's tags named nightly-*, oldest first.
nightly_tags() {
  gh api --paginate "repos/$(repo)/git/matching-refs/tags/nightly-" --jq '.[].ref' \
    | sed 's#^refs/tags/##' | { grep -E "$NIGHTLY_RE" || true; } | sort -V
}

# The repository's nightly-* pre-releases, oldest first.
nightly_releases() {
  gh release list -R "$(repo)" --limit 1000 --exclude-drafts --json tagName,isPrerelease \
    --jq '.[] | select(.isPrerelease) | .tagName' \
    | { grep -E "$NIGHTLY_RE" || true; } | sort -V
}

out() { printf '%s\n' "$@" >>"${GITHUB_OUTPUT:-/dev/stdout}"; }
summary() { printf '%s\n' "$@" >>"${GITHUB_STEP_SUMMARY:-/dev/stderr}"; }

# plan: whether this run builds, and under which tag and version. Reads
# GITHUB_REF and GITHUB_SHA (main's head when the run started) and FORCE.
plan() {
  local sha=${GITHUB_SHA:?} ref=${GITHUB_REF:?} prev prev_sha="" day tag n taken version
  [ "$ref" = refs/heads/main ] || die "a nightly is built from main; this run is on $ref"
  prev="$(nightly_releases | tail -n 1)"
  if [ -n "$prev" ]; then
    prev_sha="$(gh api "repos/$(repo)/commits/$prev" --jq .sha)"
  fi
  if [ -n "$prev" ] && [ "$prev_sha" = "$sha" ] && [ "${FORCE:-false}" != true ]; then
    summary "main has not moved since $prev (${sha:0:7}): no nightly this run."
    out build=false
    return 0
  fi
  day="$(date -u +%Y-%m-%d)"
  taken="$(nightly_tags)"
  tag="nightly-$day"
  n=1
  while grep -qxF "$tag" <<<"$taken"; do
    n=$((n + 1))
    tag="nightly-$day.$n"
  done
  version="$(version "$day" "$n" "$sha")"
  local image
  image="ghcr.io/$(repo)"
  local daytag="nightly-${day//-/}"
  [ "$n" -gt 1 ] && daytag="$daytag.$n"
  out build=true "tag=$tag" "version=$version" "sha=$sha"
  # the image: the moving `nightly`, the day's, and the version itself
  # (`+` is `-` there, a docker tag holds no `+`), which a nightly's
  # `hale habitat up` pulls as its own
  {
    echo 'image_tags<<EOF_TAGS'
    echo "type=raw,value=nightly"
    echo "type=raw,value=$daytag"
    echo "type=raw,value=${version//+/-}"
    echo 'EOF_TAGS'
    echo 'image_labels<<EOF_LABELS'
    echo "org.opencontainers.image.version=$version"
    echo "org.opencontainers.image.revision=$sha"
    echo 'EOF_LABELS'
  } >>"${GITHUB_OUTPUT:-/dev/stdout}"
  summary "Building $tag: hale $version from main at $sha, image $image:nightly and $image:$daytag."
  if [ -n "$prev" ]; then summary "The previous nightly is $prev (${prev_sha:0:7})."; fi
}

# The pull requests merged into main between two commits, as `- #N title`,
# from main's first-parent history: a merge's subject names the PR and its
# body's first line is the title; a squash's subject ends in `(#N)`.
merged_prs() {
  local from=$1 to=$2
  git -C "$root" log --first-parent --format='%s%x1f%b%x1e' "$from..$to" | awk -v RS='\036' -v FS='\037' '
    {
      sub(/^\n+/, "", $1)
      if (match($1, /^Merge pull request #[0-9]+/)) {
        n = substr($1, 21, RLENGTH - 20)
        t = $2; sub(/^\n+/, "", t); sub(/\n.*/, "", t)
        if (t == "") t = $1
        print "- #" n " " t
      } else if (match($1, /\(#[0-9]+\)$/)) {
        n = substr($1, RSTART + 2, RLENGTH - 3)
        t = substr($1, 1, RSTART - 1); sub(/ +$/, "", t)
        print "- #" n " " t
      }
    }'
}

# The unreleased/ fragments, by heading: each fragment's bullets cut to
# their first sentence, with the PR they came from.
fragments() {
  local f
  for f in "$root"/unreleased/[0-9]*.md; do
    [ -e "$f" ] || continue
    awk -v pr="$(basename "$f" .md)" '
      /^### / { h = substr($0, 5); next }
      /^- / {
        t = substr($0, 3)
        if (match(t, /\. /)) t = substr(t, 1, RSTART)
        if (length(t) > 200) t = substr(t, 1, 197) "..."
        print (h == "" ? "Changed" : h) "\037- " t " (#" pr ")"
      }' "$f"
  done | awk -F '\037' '
    { if (!($1 in seen)) { seen[$1] = 1; order[++k] = $1 } body[$1] = body[$1] $2 "\n" }
    END { for (i = 1; i <= k; i++) printf "### %s\n\n%s\n", order[i], body[order[i]] }'
}

# notes <tag> <version> <sha>: the release body. Needs the history back to
# the previous nightly (or, before the first, the newest `v*` release).
notes() {
  local tag=$1 version=$2 sha=$3 since prs frags daytag
  since="$(nightly_releases | grep -vxF "$tag" | tail -n 1 || true)"
  if [ -z "$since" ]; then
    since="$(gh release list -R "$(repo)" --limit 100 --exclude-drafts --json tagName --jq '.[].tagName' | grep -E '^v[0-9]' | head -n 1 || true)"
  fi
  daytag="$(sed -E 's/^nightly-([0-9]{4})-([0-9]{2})-([0-9]{2})/nightly-\1\2\3/' <<<"$tag")"
  cat <<EOF
Nightly build of \`main\` at [\`${sha:0:7}\`](https://github.com/$(repo)/commit/$sha): \`hale $version\`.

A nightly carries main's day-to-day changes between releases; releases are the milestones. Install it with

\`\`\`sh
curl -fsSL https://hale-lang.org/install.sh | HALE_VERSION=$tag sh
\`\`\`

(\`HALE_VERSION=nightly\` installs the newest nightly), or pull \`ghcr.io/$(repo):$daytag\` (\`:nightly\` moves to the newest).

EOF
  if [ -n "$since" ]; then
    git -C "$root" fetch -q --no-tags origin "+refs/tags/$since:refs/tags/$since" 2>/dev/null || true
    prs="$(merged_prs "$since" "$sha" || true)"
    echo "## Merged since $since"
    echo
    if [ -n "$prs" ]; then echo "$prs"; else echo "No pull request merged since $since."; fi
    echo
  fi
  frags="$(fragments)"
  echo "## Unreleased changes on main"
  echo
  echo "From \`unreleased/\`, the CHANGELOG fragments the next release folds:"
  echo
  if [ -n "$frags" ]; then echo "$frags"; else echo "None."; echo; fi
}

# prune [--dry-run]: delete the nightly-* pre-releases (and their tags)
# beyond the newest $KEEP, then any other nightly-* tag beyond the newest
# $KEEP that no kept release stands on.
prune() {
  local dry=false t keep_releases releases tags
  [ "${1:-}" = --dry-run ] && dry=true
  releases="$(nightly_releases)"
  keep_releases="$(tail -n "$KEEP" <<<"$releases")"
  while read -r t; do
    [ -n "$t" ] || continue
    [[ "$t" =~ $NIGHTLY_RE ]] || die "refusing to prune $t"
    echo "pruning the release $t"
    $dry || gh release delete "$t" -R "$(repo)" --cleanup-tag --yes
  done < <(all_but_newest "$releases")
  tags="$(nightly_tags)"
  while read -r t; do
    [ -n "$t" ] || continue
    [[ "$t" =~ $NIGHTLY_RE ]] || die "refusing to prune $t"
    grep -qxF "$t" <<<"$keep_releases" && continue
    # a pruned release took its tag with it
    gh api "repos/$(repo)/git/ref/tags/$t" >/dev/null 2>&1 || continue
    echo "pruning the tag $t"
    $dry || gh api -X DELETE "repos/$(repo)/git/refs/tags/$t" >/dev/null
  done < <(all_but_newest "$tags")
}

# Every line but the last $KEEP (portable: BSD head has no negative count).
all_but_newest() {
  local total
  total=$(grep -c . <<<"$1" || true)
  [ "$total" -gt "$KEEP" ] || return 0
  head -n "$((total - KEEP))" <<<"$1"
}

cmd=${1:-}
shift || true
case "$cmd" in
  version) version "$@" ;;
  set-version) set_version "$@" ;;
  plan) plan ;;
  notes) notes "$@" ;;
  prune) prune "$@" ;;
  *) die "usage: nightly.sh version|set-version|plan|notes|prune" ;;
esac
