# log-demo friction log

Append-only. Format per
`notes/agent-onboarding/app-dev-brief.md` § "The friction-log
contract".

## 2026-05-10 example-corpus contradicts brief on `import`

**Tried:** Trust the example corpus as a guide for current
syntax — specifically `examples/01-locus-with-run/main.ap`
which begins with `import "std/time";` and then calls
`time::sleep(...)`.
**Hit:** The brief's counter-hallucination table explicitly
says `import` / `use` / `from` do not exist and the only
namespace mechanism is the magic `std::*` prefix called
inline. The brief's reading-order also says examples 01-05
are canonical to skim. So the brief and the canonical
"please read this first" example disagree on whether `import`
exists.
**Workaround:** Followed the brief and `examples/05-bus/main.ap`
(the bus example, which has no `import` and uses bus subjects
directly). Skipped the time::sleep ergonomics question
entirely because this app doesn't need timing.
**Why it matters:** Cold-start agents are told to read 01-05
first. If they internalise the syntax in 01 they will write
`import "std/io";` in their first program and then fight the
parser. A 30-second update to 01/08/25/36/40-42 (or a "legacy
syntax" note in their headers) would eliminate this.
Affected files I noticed: `examples/01-locus-with-run/`,
`08-monotonic-sleep/`, `25-imports/`, `36-duration-closures/`,
`40-pinned-duration/`, `41-closure-accumulator/`,
`42-accumulator-vocab/`.

## 2026-05-10 enum levels deferred — int constants instead

**Tried:** Model log levels as
`enum Level { Info, Warn, Error }` with the sink dispatching
on the variant.
**Hit:** Per the counter-hallucination table, "match patterns
on enum variants" is deferred; `examples/43-enums` and
`45-enum-payloads` exist but the brief says payload
pattern-matching is not shipped. For a sink that fans by
level this means an `if`-chain over an `Int`, not a
variant-match.
**Workaround:** Used `level: Int` with 1/2/3 constants and an
`if`-chain in the sink. Documented the swap in the README so
it is a one-line edit when variant-matching lands.
**Why it matters:** Structured logging is one of the most
natural sum-type cases (`Level` is a closed set of variants).
This is a known deferred feature, so this entry is more
"ratifying the brief is right that it'd help" than novel
friction. Minor papercut.
