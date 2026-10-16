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

# case_ <name> <expected prose> <expected dna> <expected cli> <paths...>
case_() {
  local name=$1 ep=$2 ed=$3 ec=$4; shift 4
  git checkout -q "$base" && git checkout -q -B "t-$name"
  for f in "$@"; do mkdir -p "$(dirname "$f")"; echo "$RANDOM" >> "$f"; done
  git add -A && git commit -qm "$name"
  got=$(bash "$here/ci-scope.sh" "$base" HEAD 2>/dev/null | tr '\n' ' ')
  want="prose=$ep dna=$ed cli=$ec "
  if [ "$got" = "$want" ]; then echo "ok   $name: $got"; else echo "FAIL $name: got '$got' want '$want'"; fail=1; fi
}

case_ docs-only   true  false false docs/a.md README.md
case_ unreleased  true  false false unreleased/1.md
case_ tests-only  false false false crates/hale-codegen/tests/t.rs
case_ stdlib-edit false false false crates/hale-stdlib/hl/s.hl
case_ codegen-src  false false false crates/hale-codegen/src/x.rs
case_ cargo-lock  false false true  Cargo.lock
case_ dna-edit    false true true  dna/a.hl
case_ workflow    false true true  .github/workflows/tests.yml
case_ dna-suite   false true true  crates/hale-cli/tests/dna_native_suite.rs

case_ nextest-cfg false true true  .config/nextest.toml
case_ ts-fixture  false true true  crates/hale-cli/tests/fixtures/ts-client/run.mjs
case_ script      false true true  scripts/other.sh
case_ contract    false true true  tests/api-contract/program.hl
case_ replay      false true true  tests/hale/api/client/replay/main.hl
case_ dna-host    false true  true  crates/hale-cli/src/dna.rs
case_ dna-host-dir false true true  crates/hale-cli/src/dna/verbs.rs
case_ iris-crate  false true  true  crates/hale-iris/src/lib.rs
case_ dna-crate   false true  true  crates/hale-dna/build.rs
case_ fleet-src   false true  true  crates/hale-cli/src/fleet_model.rs
case_ dna-cli-test false true true  crates/hale-cli/tests/dna_orgs.rs
case_ api-gen     false true  true  crates/hale-cli/src/api_gen/ts_client.rs
case_ cli-src     false false true  crates/hale-cli/src/main.rs
case_ lsp-src     false false true  crates/hale-lsp/src/lib.rs
case_ cli-test    false false true  crates/hale-cli/tests/tooling.rs
case_ hale-test   false false true  tests/hale/other_test.hl

# a rename out of dna/ removes a source the fixtures import: both sides count
git checkout -q "$base" && git checkout -q -B t-rename
mkdir -p experiments && git mv dna/a.hl experiments/a.hl && git commit -qm rename
got=$(bash "$here/ci-scope.sh" "$base" HEAD 2>/dev/null | tr '\n' ' ')
if [ "$got" = "prose=false dna=true cli=true " ]; then echo "ok   rename-out-of-dna: $got"; else echo "FAIL rename-out-of-dna: got '$got'"; fail=1; fi

got=$(bash "$here/ci-scope.sh" "" HEAD 2>/dev/null | tr '\n' ' ')
if [ "$got" = "prose=false dna=true cli=true " ]; then echo "ok   no-base: $got"; else echo "FAIL no-base: $got"; fail=1; fi
got=$(bash "$here/ci-scope.sh" "$base" "$base" 2>/dev/null | tr '\n' ' ')
if [ "$got" = "prose=false dna=true cli=true " ]; then echo "ok   empty: $got"; else echo "FAIL empty: $got"; fail=1; fi
exit $fail
