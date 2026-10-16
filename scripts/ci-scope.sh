#!/usr/bin/env bash
# Classify a diff for the `scope` job of .github/workflows/tests.yml.
#
#   scripts/ci-scope.sh <base> <head>
#
# prints, on stdout, one line each:
#   prose=true|false  every changed path is prose (docs, spec, notes, ...)
#   dna=true|false    the diff touches DNA's own tree, the host code that
#                     runs it, or what its jobs run: the DNA and face
#                     jobs run on the pull request
#   cli=true|false    the diff touches the cli family (hale-cli and the
#                     crates only it builds, tests/hale, the manifests),
#                     or dna is true: the cli jobs run on the pull request
# The merge queue and main's push run every job whatever these say.
#
# Fails SAFE: an unusable base or head, or an empty diff, is
# prose=false dna=true cli=true. The workflow redirects the output into
# $GITHUB_OUTPUT; run it by hand to see what a branch would trigger.
set -uo pipefail

base="${1:-}"
head="${2:-HEAD}"

verdict() { echo "prose=$1"; echo "dna=$2"; echo "cli=$3"; }

if [ -z "$base" ] || ! git cat-file -e "$base^{commit}" 2>/dev/null \
  || ! git cat-file -e "$head^{commit}" 2>/dev/null; then
  echo "ci-scope: no usable base, classifying as everything" >&2
  verdict false true true; exit 0
fi

if ! changed=$(git diff --name-only --no-renames "$base"..."$head"); then
  echo "ci-scope: diff failed, classifying as everything" >&2
  verdict false true true; exit 0
fi
if [ -z "$changed" ]; then
  echo "ci-scope: empty diff, classifying as everything" >&2
  verdict false true true; exit 0
fi
echo "changed files:" >&2; echo "$changed" >&2

# Paths that cannot reach the runtime, the models, or codegen.
prose='^(README|AGENTS|CHANGELOG|CONTRIBUTING|LICENSE|SECURITY)|^(docs|spec|notes|agents|unreleased)/|\.md$'

# Paths that run the DNA and face jobs on a pull request: DNA's own tree
# and the iris tree it embeds; the host code that runs them (hale-dna,
# hale-iris, hale-cli's dna, iris, fleet and mcp verbs) and the cli
# tests that drive it; the generated API clients (api_gen/), which only
# face-browser compiles under `tsc --strict` and runs; the workflows, .config and scripts/; the API
# contract programs; and the DNA suite with its support code. A change
# to the compiler's other crates does not: the merge queue runs every
# job on every PR before anything reaches main (2026-10-16).
dna='^(dna|iris|\.github/workflows|\.config|scripts|tests/api-contract|tests/hale/api/client)/|^crates/(hale-dna|hale-iris)/|^crates/hale-cli/src/(dna/|api_gen/|dna\.rs$|iris\.rs$|fleet[a-z_]*\.rs$|mcp\.rs$)|^crates/hale-cli/tests/dna_'
dna_suite='^crates/hale-cli/tests/(dna_native_suite\.rs|support/|fixtures/)'
# Paths that run the cli jobs on a pull request (as does any DNA path):
# hale-cli and the crates only it builds, the in-language suite the cli
# jobs run (tests/hale), and the workspace manifests.
cli='^crates/(hale-cli|hale-lsp|hale-frontend|hale-ts-shim)/|^tests/hale/|^Cargo\.(toml|lock)$'

nonprose=$(printf '%s\n' "$changed" | grep -cvE "$prose" || true)
hit=$(printf '%s\n' "$changed" | grep -cE "$dna" || true)
suite=$(printf '%s\n' "$changed" | grep -cE "$dna_suite" || true)
clihit=$(printf '%s\n' "$changed" | grep -cE "$cli" || true)

p=false; [ "${nonprose:-1}" -eq 0 ] && p=true
d=false
if [ "${hit:-1}" -gt 0 ] || [ "${suite:-0}" -gt 0 ]; then d=true; fi
c=$d
if [ "${clihit:-1}" -gt 0 ]; then c=true; fi
verdict "$p" "$d" "$c"
