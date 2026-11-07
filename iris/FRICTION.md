# Iris implementation friction

Re-read against hale v0.32.7 (main `7764d0e18`, 2026-11-06). Each entry
carries its status; the text under it is the observation as made.

## F.1 — Copied String fields in the intake-control proof fixture

**Status (2026-11-06):** FIXED upstream in hale v0.24.0 (#1550, #1558): a value you read is yours. A local bound from a field, element, local or parameter copies at its first write, so mutating `copy` no longer changes what it was read from. No Iris change needed; the proof's independent literals are still fine.

The installed compiler accepts `let mut copy = original` for a shape with String
fields, but the first proof's mutation of that copy also changed strings later
read from the original request/state. This caused the test to submit a different
payload while expecting idempotent recovery. The proof now constructs each
request and receipt as an independent literal and does not mutate aliases.
Production provider requests/receipts are immutable values. No compiler change
or general lifetime repair is claimed.

## F.3 — Native node delivery carries the request, not its audit envelope

**Status (2026-11-06):** documented upstream. spec/stdlib.md (`std::process`) names the 64 KiB argv cap (`LOTUS_PROCESS_ARGV_MAX`), that a longer blob fails `run`/`spawn` with `IoError { kind: "invalid", errno: 7 }` before any fork, and says to pass bulk data on stdin (`spawn` + `write_stdin` + `close_stdin`, the last added 2026-10-12) or in a file. The host's eleven-field request stays the right shape.

The installed process launcher bounds newline-separated argv at 65536 bytes.
A valid 8192-byte Knowledge text with JSON-escaped controls produced a 94888-byte
admission fact because that fact also retains its canonical command payload.
Passing the entire fact to the membrane failed before the membrane was launched.
The host now sends only the exact eleven native request fields after its existing
ownership and signature checks. The complete audit envelope stays in the Record.
This changes neither durable request identity nor domain admission.

## F.4 — Preserve exact large Knowledge text through encoding and projection

**Status (2026-11-06):** no upstream ask; a domain adaptation. The quadratic JSON escaping it worked around upstream (GH #708, #745; runs appended rather than bytes in #1524) is linear now.

A control-heavy maximum-size candidate exposed a failure in incremental JSON
construction on the native Body path. Knowledge document construction now uses
the existing linear byte builder and exact JSON quoting; the large Review
question follows the same quoting path. Small historical-encoder parity and a
fixed legacy digest guard the stored identity. A real 8192-byte Body/Review and
graph-replay test verifies the large case under the existing resource bounds.
The graph tail also uses the native exact string decoder so escaped non-NUL
control bytes are preserved. These are bounded domain/read adaptations; no
compiler or stdlib repair is included.

## F.2 — Names and scoped statement cleanup

**Status (2026-11-06):** PARTLY FIXED upstream in hale v0.24.0 (#1531): `capacity`, `publish`, `restart` and the other framework words are contextual keywords and can name locals, record fields, free fns and methods. Still refused on v0.32.7 (verified with `hale check`): a locus method cannot be named `run`, `drain` or `release` (those names are the lifecycle hooks) or `draining` (the drain flag every locus carries), and `restart` cannot name a free fn (at statement head it is the recovery statement). The renames here can stay; a method named for a hook must keep its rename.

`restart` and `capacity` are reserved Hale words; local bindings and methods use
other names. Prepared-statement cleanup is implemented by a scoped locus, not by
assuming every branch reaches an explicit finalize call.
