# Iris implementation friction

## F.1 — Copied String fields in the intake-control proof fixture

The installed compiler accepts `let mut copy = original` for a shape with String
fields, but the first proof's mutation of that copy also changed strings later
read from the original request/state. This caused the test to submit a different
payload while expecting idempotent recovery. The proof now constructs each
request and receipt as an independent literal and does not mutate aliases.
Production provider requests/receipts are immutable values. No compiler change
or general lifetime repair is claimed.

## F.3 — Native node delivery carries the request, not its audit envelope

The installed process launcher bounds newline-separated argv at 65536 bytes.
A valid 8192-byte Knowledge text with JSON-escaped controls produced a 94888-byte
admission fact because that fact also retains its canonical command payload.
Passing the entire fact to the membrane failed before the membrane was launched.
The host now sends only the exact eleven native request fields after its existing
ownership and signature checks. The complete audit envelope stays in the Record.
This changes neither durable request identity nor domain admission.

## F.4 — Preserve exact large Knowledge text through encoding and projection

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

`restart` and `capacity` are reserved Hale words; local bindings and methods use
other names. Prepared-statement cleanup is implemented by a scoped locus, not by
assuming every branch reaches an explicit finalize call.
