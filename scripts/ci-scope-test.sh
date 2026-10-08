#!/usr/bin/env bash
# Runs scripts/ci-scope.sh on synthetic diffs and checks both outputs.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
cd "$tmp" || exit 1
git init -q . && git config user.email t@t && git config user.name t
mkdir -p docs dna crates/hale-stdlib/hl crates/hale-codegen/tests
echo a > README.md; echo a > docs/a.md; echo a > dna/a.hl
echo a > crates/hale-stdlib/hl/s.hl; echo a > crates/hale-codegen/tests/t.rs
git add -A && git commit -qm base
base=$(git rev-parse HEAD)
fail=0

# case_ <name> <expected prose> <expected dna> <paths...>
case_() {
  local name=$1 ep=$2 ed=$3; shift 3
  git checkout -q "$base" && git checkout -q -B "t-$name"
  for f in "$@"; do mkdir -p "$(dirname "$f")"; echo "$RANDOM" >> "$f"; done
  git add -A && git commit -qm "$name"
  got=$(bash "$here/ci-scope.sh" "$base" HEAD 2>/dev/null | tr '\n' ' ')
  want="prose=$ep dna=$ed "
  if [ "$got" = "$want" ]; then echo "ok   $name: $got"; else echo "FAIL $name: got '$got' want '$want'"; fail=1; fi
}

case_ docs-only   true  false docs/a.md README.md
case_ unreleased  true  false unreleased/1.md
case_ tests-only  false false crates/hale-codegen/tests/t.rs
case_ stdlib-edit false true  crates/hale-stdlib/hl/s.hl
case_ dna-edit    false true  dna/a.hl
case_ workflow    false true  .github/workflows/tests.yml
case_ dna-suite   false true  crates/hale-cli/tests/dna_native_suite.rs

case_ nextest-cfg false true  .config/nextest.toml
case_ ts-fixture  false true  crates/hale-cli/tests/fixtures/ts-client/run.mjs
case_ script      false true  scripts/other.sh
case_ contract    false true  tests/api-contract/program.hl
case_ replay      false true  tests/hale/api/client/replay/main.hl
case_ hale-test   false false tests/hale/other_test.hl

# a rename out of dna/ removes a source the fixtures import: both sides count
git checkout -q "$base" && git checkout -q -B t-rename
mkdir -p experiments && git mv dna/a.hl experiments/a.hl && git commit -qm rename
got=$(bash "$here/ci-scope.sh" "$base" HEAD 2>/dev/null | tr '\n' ' ')
if [ "$got" = "prose=false dna=true " ]; then echo "ok   rename-out-of-dna: $got"; else echo "FAIL rename-out-of-dna: got '$got'"; fail=1; fi

got=$(bash "$here/ci-scope.sh" "" HEAD 2>/dev/null | tr '\n' ' ')
if [ "$got" = "prose=false dna=true " ]; then echo "ok   no-base: $got"; else echo "FAIL no-base: $got"; fail=1; fi
got=$(bash "$here/ci-scope.sh" "$base" "$base" 2>/dev/null | tr '\n' ' ')
if [ "$got" = "prose=false dna=true " ]; then echo "ok   empty: $got"; else echo "FAIL empty: $got"; fail=1; fi
exit $fail
