# iris → hale: friction report (v0.17.0)

From a full downstream pass: rebuilt and re-ran the whole iris
stack on `main` @ `84666ff`. **Everything builds, checks, verifies
and runs clean** — no breakage from the nominal-stdlib-types
change or anything else this week. What follows is friction only.

Repros are single files unless noted; all verified 2026-08-14.

| # | Friction | Severity |
|---|---|---|
| P27 | `hale check` does not detect undefined identifiers | **high** — check/build divergence, invisible to LSP and `verify` |
| P28 | Some diagnostics are emitted twice, including in `--json` | medium — duplicated squiggles in editors |
| P29 | `std::str::len` doesn't point at the builtin `len()` | low |
| P30 | `replay --diff` reports "matches" with equal confidence on an empty schedule | medium — new surface, common app shape |

---

## P27 — an undefined identifier passes `check`, `--json` and `verify`, then fails codegen

The most basic check there is, and nothing catches it until
codegen.

```
$ cat main.hl
fn main() {
    let total = 1;
    println("" + totl);      // typo
}

$ hale check .        →  ok: 1 file(s) typechecked
$ hale check . --json →  (empty — no diagnostic at all)
$ hale verify .       →  verified: 1 file(s), 0 findings
$ hale build .        →  codegen error: unsupported in codegen v0:
                         unknown identifier `totl`
```

Three separate consequences, and each is worse than the last:

1. **No editor squiggle.** `--json` is the LSP path, so a typo'd
   identifier is invisible while you type and appears only when
   you build.
2. **`verify` passes.** The discipline gate reports zero findings
   on a program that cannot be compiled.
3. **The build error has no location** — no file, no line, no
   column, so it is not clickable and not greppable to a site.
   On a large seed you get the identifier name and nothing else.

This is the check/build divergence class the changelog itself
calls out ("the class that lets a checked-green library fail to
ship", the implicit block-tail return entry). Same shape, but for
undefined names rather than a lowering corner.

**Uniform across every position tried** — not an edge case. Each
of these checks clean and dies in codegen:

```hale
println("" + nope);                  // concat operand
let x = nope;                        // let RHS
let x: Int = nope;                   // let RHS, ascribed
f(nope);                             // call argument
if nope { }                          // if condition
a = nope;                            // assignment RHS
```

Single-file and directory/seed mode both. Codegen clearly knows
the name is unbound; the resolver just never says so.

**Ask:** raise unbound identifiers in `check`, at the
identifier's own span. If the full fix is larger than it looks,
the cheap intermediate is giving the codegen error a span — an
unlocated error on a real codebase is the expensive part.

## P28 — a diagnostic is emitted twice, `--json` included

```
$ cat main.hl
type T { a: Int = 0; }
fn main() { let t = T { }; println("" + t.b); }

$ hale check main.hl
main.hl:2:41: type error: no field `b` on `T` — did you mean `a`?
    fn main() { let t = T { }; println("" + t.b); }
                                            ^^^
main.hl:2:41: type error: no field `b` on `T` — did you mean `a`?
    fn main() { let t = T { }; println("" + t.b); }
                                            ^^^
```

Byte-identical, same span, twice. `--json` emits two identical
NDJSON objects, so an editor renders two overlapping diagnostics
on one token.

Affected / not affected, from a small sweep:

| diagnostic | copies |
|---|---|
| `no field X on T` | **2** |
| `unknown stdlib function` | **2** |
| type mismatch (`expected X, got Y`) | 1 |
| unknown namespace (`std::nope::thing`) | 1 |

`e044250` fixed the `run`/`build` double-report; this looks like
a surviving sibling on the `check` path for a subset of kinds.

## P29 — `std::str::len` doesn't point at `len()`

```
$ hale check -   # fn main() { let s = "a"; println("" + std::str::len(s)); }
type error: unknown stdlib function `std::str::len`
```

The answer is the builtin `len(s)`, which the diagnostic doesn't
mention. The guess is natural because every *other* string
operation is `std::str::*` (`index_of`, `substring`, `trim`,
`from_bytes`), so `len` looks like the odd one out rather than a
different namespace. Cost a real round-trip while writing a
consumer.

Worth noting the neighbouring diagnostics are good — `no field
b on T — did you mean a?` is exactly right — so this is a missing
suggestion, not a systemic gap.

**Ask:** did-you-mean from `std::str::len` (and any other
stdlib-shaped miss with a builtin answer) to the builtin.

## P30 — `replay --diff` says "matches" as confidently on an empty schedule as on a full one

An app whose bus traffic is all intra-tree direct dispatch
records **zero consumes**, and the verdict is worded the same as
a genuinely verified replay:

```
replay matches the recording: 0 consumes across 0 consumers;
canonical payloads identical, raw ABI payload sizes matched
(88 ring records, 0 journal reads)
```

That run had 2 subscribers and 20 publishes. Adding
`placement { l: pinned; }` — making one delivery genuinely
queued — gives `20 consumes across 1 consumers` from the same
program, so the boundary is real and by design: consumes are
recorded for *queued* deliveries, and direct dispatch has no
schedule to order.

The friction is the reporting, not the boundary. The chapter
notes direct dispatch is "the devirtualized flavor most intra-app
traffic compiles to" — so the **common** app shape gets a
strong-sounding "matches" verdict in which the schedule half of
the guarantee was never exercised. A reader takes "matches the
recording" as "replay is verified for this program"; here it
means "payloads matched, and there was no schedule to check."

You clearly already care about this class — the phase-6 review
rounds mention dry-tape classification and "real proof surface"
— so this may just be a wording pass.

**Ask:** distinguish the empty-schedule case in the verdict.
Something as small as *"0 consumes — this program's traffic is
all direct dispatch; no schedule was exercised"* would keep the
line honest.

---

## Exercised and clean (no action — recorded so coverage is legible)

- The **nominal stdlib types** change (GH #470) reads as a
  straight improvement downstream: misusing a stdlib value now
  gives `expected String, got Bytes` at the right span, where the
  fail-open would have accepted it. No friction found.
- **Record & replay** works end-to-end first try, including the
  fail-closed refusal on live effects — which names the reason
  and the flag (`--allow-live-effects`) rather than just
  refusing. Good.
- The **pre-stable artifact format** caveat is documented in the
  chapter; noted, not filed. iris will not build on `.halerec`
  until you say it is stable — flagging only because iris's own
  M3 is a flight recorder, so we would rather consume yours than
  ship a second format. Happy to be a design consumer whenever
  that phase comes up.
- The full iris stack on 0.17.0: `check`, `verify` (0 findings),
  `build`, and a live end-to-end run (observed app → fuse-hl →
  inspector, model identity and per-topic hashes both in sync).
