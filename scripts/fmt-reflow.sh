#!/usr/bin/env bash
# The whole-tree reflow for the 100-column width rule (spec/testing.md
# § `hale fmt`, Width). Run it once, in a quiet window, on a branch
# whose HEAD carries "fmt: wrap at 100 columns by default": that commit
# makes wrapping canonical, and this reformats the tree to match in ONE
# commit, so main's `hale fmt --check .` is never red across the pair.
#
#   scripts/fmt-reflow.sh            reflow, re-pin, verify, commit
#   scripts/fmt-reflow.sh --dry-run  reflow and report; commit nothing
#
# What it does, in order, stopping at the first failure:
#   1. refuses a dirty tree, and a HEAD whose `hale fmt` does not wrap;
#   2. builds the workspace (the binary formats; the stdlib and DNA it
#      embeds are re-read after the reflow, so it builds again at 6);
#   3. `hale fmt .` over the tree, then `hale fmt --check .` as the fixed
#      point: a second pass must change nothing;
#   4. every `.hl` that parsed before the reflow parses after it;
#   5. rebuilds, so `hale` embeds the reflowed stdlib and DNA;
#   6. regenerates the two pins keyed on a file's text that a reflow
#      moves and that have a regeneration knob: the placement golden and
#      the target-capability shadow (HALE_SHADOW_REGEN=1);
#   7. commits everything as one commit.
#
# Measured on 2026-10-19 (a full `cargo nextest run --workspace` over the
# reflowed tree), the reflow also moves pins and tests that have no knob.
# Each one is keyed on a byte offset, a line number or a source line's
# exact text, and none of them on what a program means. They are edited
# by hand in the same PR:
#   - hale-types alloc_summary_construction_correction
#     (own_entries_run_to_exit_programs, reached_oidc) and
#     alloc_summary_correction (verdict_changes): byte offsets;
#   - hale-cli check_units::the_committed_forms_report_is_pinned_whole:
#     line numbers;
#   - tests that edit tests/api-contract/program.hl with a text
#     replacement the reflow split over lines: api_pb_codec (2),
#     api_witness_whole (2), and
#     api_contract_fixtures::a_grpc_exposure_is_described_with_the_grpc_outcomes;
#   - hale-cli dna_projects tests that match the text `hale dna init`
#     and `dna new` generate from reflowed templates: dna_models::init_writes_the_catalog_…,
#     dna_new::new_makes_a_governed_application_…,
#     dna_init::upgrade_retires_the_owners_map.
# The PR's CI is the gate for everything else.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

dry=0
[ "${1:-}" = "--dry-run" ] && dry=1

if [ -n "$(git status --porcelain)" ]; then
    echo "fmt-reflow: the tree is dirty; commit or stash first" >&2
    exit 1
fi

target=${CARGO_TARGET_DIR:-target}
hale="$target/release/hale"

echo "== build"
cargo build --release

probe=$(printf 'fn main() {\n    let r = f(aaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbb, cccccccccccccccccccc, dddddddddddddddddddd);\n}\n' \
    | "$hale" fmt --stdin | wc -l | tr -d ' ')
if [ "$probe" -le 3 ]; then
    echo "fmt-reflow: this hale does not wrap; check out the commit that makes wrapping the default" >&2
    exit 1
fi

# The files `hale fmt .` visits, and which of them parse now.
list=$(mktemp)
parsed=$(mktemp)
trap 'rm -f "$list" "$parsed"' EXIT
git ls-files '*.hl' | grep -v -E '(^|/)vendor/|(^|/)\.' > "$list"
while IFS= read -r f; do
    if "$hale" parse "$f" > /dev/null 2>&1; then
        echo "$f" >> "$parsed"
    fi
done < "$list"

echo "== hale fmt ."
if ! "$hale" fmt . > /dev/null; then
    echo "fmt-reflow: hale fmt refused a file (it names it above)" >&2
    exit 1
fi
echo "== hale fmt --check . (fixed point)"
"$hale" fmt --check .

echo "== parses as before"
broken=0
while IFS= read -r f; do
    if ! "$hale" parse "$f" > /dev/null 2>&1; then
        echo "fmt-reflow: $f parsed before the reflow and does not now" >&2
        broken=1
    fi
done < "$parsed"
[ "$broken" = 0 ] || exit 1

echo "== the reflow"
git diff --shortstat
git diff --numstat | awk '{ n++ } END { print n " files" }'

if [ "$dry" = 1 ]; then
    echo "fmt-reflow: --dry-run, the tree is left reflowed and uncommitted (git checkout -- . undoes it)"
    exit 0
fi

echo "== rebuild (the embedded stdlib and DNA)"
cargo build --release

echo "== re-pin the placement golden and the capability shadow"
cargo test --release -p hale-types --test placement_golden \
    --config 'env.HALE_SHADOW_REGEN="1"'
cargo test --release -p hale-types --test shadow_capability \
    --config 'env.HALE_SHADOW_REGEN="1"'
git diff --stat -- crates/hale-types/tests/fixtures

git add -A
git commit -q -m "fmt: reflow the tree at 100 columns

hale fmt . with the width rule (spec/testing.md, Width) over every
.hl file it visits; nothing else changes. The placement golden
and the capability shadow, keyed on file text, are regenerated with
it."
git log --oneline -1
