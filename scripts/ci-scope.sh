#!/usr/bin/env bash
# Classify a diff for the `scope` job of .github/workflows/tests.yml.
#
#   scripts/ci-scope.sh <base> <head>
#
# prints, on stdout, one line each:
#   prose=true|false  every changed path is prose (docs, spec, notes, ...)
#   dna=true|false    the diff touches DNA's own tree or what its jobs
#                     run: the DNA, face and cli jobs run on the pull
#                     request (the merge queue runs them for every PR)
#
# Fails SAFE: an unusable base or head, or an empty diff, is
# prose=false dna=true. The workflow redirects the output into
# $GITHUB_OUTPUT; run it by hand to see what a branch would trigger.
set -uo pipefail

base="${1:-}"
head="${2:-HEAD}"

verdict() { echo "prose=$1"; echo "dna=$2"; }

if [ -z "$base" ] || ! git cat-file -e "$base^{commit}" 2>/dev/null \
  || ! git cat-file -e "$head^{commit}" 2>/dev/null; then
  echo "ci-scope: no usable base, classifying as everything" >&2
  verdict false true; exit 0
fi

if ! changed=$(git diff --name-only --no-renames "$base"..."$head"); then
  echo "ci-scope: diff failed, classifying as everything" >&2
  verdict false true; exit 0
fi
if [ -z "$changed" ]; then
  echo "ci-scope: empty diff, classifying as everything" >&2
  verdict false true; exit 0
fi
echo "changed files:" >&2; echo "$changed" >&2

# Paths that cannot reach the runtime, the models, or codegen.
prose='^(README|AGENTS|CHANGELOG|CONTRIBUTING|LICENSE|SECURITY)|^(docs|spec|notes|agents|unreleased)/|\.md$'

# Paths that run the DNA, face and cli jobs on a pull request: DNA's own
# tree and the iris tree it embeds, the workflows, the scripts those jobs
# call, the API contract programs, and the DNA suite with its support
# code. A compiler change (any crate) does not: the merge queue runs
# every job on every PR before anything reaches main, so a crate PR's
# own run is the compiler suites alone and lands in about half the time
# (2026-10-16). Fails safe as before: no usable diff runs everything.
dna='^(dna|iris|\.github/workflows|\.config|scripts|tests/api-contract|tests/hale/api/client)/'
dna_suite='^crates/hale-cli/tests/(dna_native_suite\.rs|support/|fixtures/)'

nonprose=$(printf '%s\n' "$changed" | grep -cvE "$prose" || true)
hit=$(printf '%s\n' "$changed" | grep -cE "$dna" || true)
suite=$(printf '%s\n' "$changed" | grep -cE "$dna_suite" || true)

p=false; [ "${nonprose:-1}" -eq 0 ] && p=true
d=false
if [ "${hit:-1}" -gt 0 ] || [ "${suite:-0}" -gt 0 ]; then d=true; fi
verdict "$p" "$d"
