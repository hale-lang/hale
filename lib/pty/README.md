# lib/pty — stub seed

Stub implementation of the `std::pty` surface documented in
[`../../COMPILER_FFI.md`](../../COMPILER_FFI.md).

## What this is

Same Hale surface as the real `std::pty::*` will expose. Instead
of `forkpty`-backed I/O, the stub holds two in-memory
`BytesBuilder`s — `stub_output` (what the "PTY" produces) and
`stub_input` (what the user wrote to it). Tests drive both.

This lets Iris's `ShellPane` exercise its end-to-end behavior
(PTY read loop drives scrollback; user keystrokes get forwarded
to the PTY) without any OS process actually running.

## Stub helpers (test-only)

In `stub_helpers.hl`. Deleted when real bindings land.

```hale
fn stub_inject_output(p: Pty, bytes: Bytes);
   // Append `bytes` to the pty's stub_output buffer. The next
   // `read_nonblocking` call will see them.

fn stub_drain_input(p: Pty) -> Bytes;
   // Returns everything `write` has accumulated since the last
   // drain, then clears the buffer.

fn stub_set_alive(p: Pty, alive: Bool);
   // Toggle the alive flag. When false, read/write fail with
   // PtyError { kind: "closed" }.
```

## Migration

When `std::pty::*` ships:

1. Delete `stub_helpers.hl`.
2. Delete `pty.hl`'s `stub_*` params and the stub bodies.
3. Either delete the seed and switch Iris imports to `std::pty::*`,
   or keep it as a thin re-export shim.
4. Rewrite any tests that depended on stub injection to use
   a real subprocess (`echo`, `cat`, etc.) as a fixture.

## Files

- `pty.hl` — `PtyOptions`, `PtyError`, `Pty` locus, free fns.
- `stub_helpers.hl` — test injection helpers.
