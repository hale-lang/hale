# lib/sse — stub seed

Stub implementation of the `std::sse` surface documented in
[`../../COMPILER_FFI.md`](../../COMPILER_FFI.md).

## What this is

Server-Sent Events stream surface. SSE is **pure Hale** — no FFI
needed — so this stub is closer to "a not-yet-built pure-Hale
library" than to the raylib / pty stubs. We ship a stub today so
Iris's AgentHarness can be built end-to-end against a controllable
event queue; the real implementation lands once `std::http::*` is
confirmed to expose streaming-recv.

The stub uses an in-memory queue of pre-staged `SseEvent`s. Tests
push events with `stub_enqueue_event`; the `next()` call dequeues
in FIFO order. When the queue is empty and the stream is closed,
`next()` fails with `kind: "closed"`.

## Stub helpers (test-only)

In `stub_helpers.hl`. Survives the migration only if the real impl
keeps a way to drive event injection for tests — otherwise deleted.

```hale
fn stub_enqueue_event(s: SseStream, event: String, data: String);
   // Push a new SseEvent { event, data, id: "" } onto the stream's
   // queue. The next call to `next` will receive it.

fn stub_close(s: SseStream);
   // Mark the stream closed. Once the queue empties, next() fails.
```

## Migration

When `std::http::*` streaming-recv is confirmed and the real SSE
parser is implemented (likely in pure Hale, no compiler change):

1. Replace `sse.hl`'s queue-driven `next()` with a real
   `\n\n`-delimited line accumulator over `std::http::Response`.
2. Delete `stub_helpers.hl` (or rewrite it to wrap a real
   in-memory `http::Response` if we want to keep the test surface).

## Files

- `sse.hl` — `SseEvent`, `SseStream`, `next()` fn.
- `stub_helpers.hl` — test injection helpers.
