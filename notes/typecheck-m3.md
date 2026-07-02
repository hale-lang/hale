# Typecheck Milestone 3 — full-fidelity checking

Status: stages 1 + 2 + 4 SHIPPED, stage 3 FULLY SHIPPED (tranche 1: generic fn calls; tranche 2: generic struct literals + Box<Int>↔Box_Int unification — also fixed generic structs being CLI-unusable); 5 remains (+ stage-2 tranche 3: json/http sigs). The public-launch gate: errors fire at
typecheck with source spans, not two phases later at codegen/link,
and never as runtime corruption.

## Where Ty::Unknown leaks today (audited)

- `std::<ns>::<fn>(...)` path-calls type as `Ty::Unknown`, which is
  bidirectionally assignable — a typo'd stdlib call compiles into a
  codegen error (best case) or confusion. check.rs has ~41 Unknown
  touchpoints; the permissiveness is deliberate and load-bearing
  (don't break it wholesale).
- Generic templates: instantiations are validated NOWHERE at
  typecheck; monomorphization happens at codegen (`m61`/`m62`
  on-demand synthesis), so a type-arg mismatch surfaces as a codegen
  error without a span, or worse.
- Contract compatibility (F.8): spec says child's exposed type must
  match parent's consume; v0 "requires equality" but enforcement is
  partial.
- Fragmented stdlib knowledge that ALREADY exists in hale-types and
  should seed the real table: `resolve.rs` knows which paths are
  fallible and with which error SHAPE (`mark_stdlib_error_from_path`,
  `check_stdlib_error_shadowing` — ParseError/IoError/CryptoError/
  IndexError field layouts); `check.rs` knows blocking paths
  (`BLOCKING_STDLIB_PATHS`) and long-running stdlib loci; the wasm
  gate knows POSIX-only namespaces.

## Staging (each stage independently shippable)

### Stage 1 — name-level validation (typo detection)

A per-namespace ALLOWLIST of function names (no signatures). For
`std::<ns>::<fn>` where `<ns>` is tabled: unknown `<fn>` → hard
error with a did-you-mean (edit-distance over the namespace's
names). Untabled namespaces keep the Unknown fallback.

Why names first: a wrong name entry produces an easy-to-diagnose
false "unknown fn", while a wrong SIGNATURE entry produces a false
type mismatch on valid code — strictly worse. Names are also
mechanically extractable.

Extraction: codegen's dispatch lives in
`crates/hale-codegen/src/stdlib/*.rs` (22 files) — mixed match-arms
and if-chains, so extraction is per-file archaeology, not one grep.
Do it file-by-file against `spec/stdlib.md`'s module-surface table
(the spec rows are the contract; the dispatch is the truth — diff
them and fix the spec where they disagree, which is itself value).
VALIDATION GATE: `hale check` over every program in hale/apps (via
the corpus), all of pond, all of fathom — zero new errors before the
stage ships.

### Stage 2 — signature table for the scalar-heavy namespaces — SHIPPED 2026-07-02

118 rows shipped (math/time/env/decimal/process-scalar/str/stdin/
stdout/bytes/crypto/base64/rand); tranche 2 = io::fs/file/tcp/tls/
udp + process child-management + json/text. Excluded-not-guessed:
str::builder_* (opaque handles), can_parse_decimal (SPEC BUG: listed
in stdlib.md, absent from dispatch). Original plan follows.

Full `(params, ret, fallible)` rows for the namespaces where
signatures are mostly scalars and the table is low-risk:
`std::math`, `std::time`, `std::env`, `std::process` (scalar
subset), `std::decimal`, `std::str` (parse/predicate/substring
family), `std::bytes` (at/read_*/write_* — all `(Bytes|BytesMut,
Int[, Int]) -> Int|Float fallible(IndexError)`). Enforce arity +
arg types + RETURN TYPE (killing Unknown for these). The
`Ty::Fallible` returns must line up with resolve.rs's existing
error-shape knowledge — unify those two sources into the one table
while there.

Then the string/pointer namespaces (io::fs, io::file, io::tcp,
text, crypto, json) in a follow-up tranche — more Bytes/String/
locus-returning shapes, more care.

### Stage 3 — generic instantiation checking

At each call site of a generic template (fn or type), infer the
concrete type args (same inference codegen's monomorphizer runs —
lift it into hale-types or mirror it), substitute into the
template's declared param/ret types, and check the call like a
non-generic one. Codegen keeps its synthesis; typecheck gains the
validation + spans. Also validate generic STRUCT literals
(`Box<Int> { ... }` field types post-substitution).

### Stage 4 — F.8 contract compatibility — SHIPPED 2026-07-02

The consume side already existed (check_contract_compatibility:
missing expose, type mismatch, consume-without-accept). What was
missing — and shipped — is the EXPOSE side: every expose entry must
bind against a real params field, mode, or fn member at a matching
type (check_contract_expose_validity). Codegen ignores contract
members entirely, so typecheck is the only enforcement point.
Bonus: mode keywords are now admitted in contract-name position
(`expose bulk: Float;`), making the exposed-mode pull rule
expressible — it was a parse error before.

### Stage 5 — memory-bounds as errors

Flip `--warn-unbounded-alloc` to default-on, and promote to error
under a `--strict-bounds` flag first. Precondition (per
spec/verification.md): a false-positive audit — run the analysis
over pond + fathom + the corpus, triage every finding as
true/false, and fix the classifier's false positives (the
store-latest scratch-accumulation verdict from the inline-arrays
work is the model: the verdict was RIGHT for a subtle reason —
document each). Only when the audit is clean on real code does the
default flip. This is the "Hale's answer to the borrow checker"
milestone and should not ship half-audited.

## Non-goals for M3

- Full HM inference — Hale's explicit-ascription posture stays.
- Interface satisfaction beyond the existing structural check.
- Locus-method receiver typing beyond what exists (stage-3-adjacent
  but separate).

## Sequencing note

Stages 1–2 are the public-launch UX ("the compiler catches my
typos and types my stdlib calls"); 3–4 close the correctness gaps;
5 is the flagship guarantee. 1, 2, and 4 are independent; 3 is the
long pole; 5 gates on an audit that can start any time.
