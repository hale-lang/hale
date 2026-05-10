# log-router friction log

Append-only. Format per the app-dev brief:

```
## YYYY-MM-DD <short tag>

**Tried:** <what you wanted to write>
**Hit:** <error / missing primitive / etc.>
**Workaround:** <what you did, or "blocked">
**Why it matters:** <feature gated, or "minor papercut">
```

## 2026-05-10 stale-cli-silent-drops-subscribers

**Tried:** Build a custom log sink (`DbAuditSinkL` subscribing to
`log.app.db.**`) with `target/debug/aperio build apps/log-router/main.ap`
and run the produced binary, expecting the `on_db` handler to
fire — same shape as `crates/aperio-codegen/tests/stdlib_log.rs`'s
`user_subscriber_can_match_subtree_pattern` test.
**Hit:** The binary built without errors, ran without errors, but
`on_db` never fired — stdout was completely empty (and the audit
file was 0 bytes). The same source pasted into the test fixture
DOES emit the expected output. Symptom looked like a real
runtime bug in user-locus bus subscription. Root cause was a
stale `target/debug/aperio` binary: `cargo test` rebuilt the
codegen+runtime crates (so the test passed against current
code) but `aperio-cli` itself wasn't rebuilt, so `aperio build`
was emitting binaries against an older lowering that quietly
dropped user-defined subscribers. `cargo build -p aperio-cli`
fixed it; everything worked on the next `aperio build`.
**Workaround:** Rebuild the CLI explicitly. The brief's "How to
run" section says `aperio run apps/<your-app>/main.ap`; the
target's run instructions in the agent prompt say `target/debug/aperio
build`. Neither prompts the agent to verify the CLI is fresh.
**Why it matters:** Silent miscompile is the worst failure
mode for a cold-context agent. There was no error message — no
"unknown locus", no "subscription dropped", no warning — the
binary just produced empty output. An agent without a working
fixture to A/B against would have spent the rest of its budget
hunting a phantom runtime bug. Either the CLI should refuse to
run when its lowering is older than the codegen crate it links
against (cheap: rebuild-on-stale check, or version stamp), or
the brief's "running and testing" section should say "if you
ever changed `crates/`, rerun `cargo build -p aperio-cli`
before `aperio build`."

## 2026-05-10 write-file-truncates-no-append

**Tried:** Write each `LogEvent` to the audit file as it arrives,
the way a "real" log sink would (open, append line, close).
**Hit:** `std::io::fs::write_file(path, content)` truncates and
takes the whole content as a String. There is no
`write_file_append`, no streaming `open`/`write`/`close`, and
no `Bytes`-incremental writer in the shipped surface (per
`docs/std/src/io/fs.md` and the brief's "what is shipped"
table).
**Workaround:** Buffer all rendered events in a `self.buf:
String` field on the sink locus. Flush once in `dissolve()`.
For this 5-event demo it's fine; a long-running sink would OOM
on a sustained event rate.
**Why it matters:** Any "log to file" sink — the canonical
custom-sink shape — is forced into one of two suboptimal
patterns: (a) buffer-everything-then-flush (memory-bounded by
process life), or (b) read-modify-write the whole file per
event (O(n²) and a huge read amplification). A
`write_file_append` overload, or a `std::io::fs::File` handle
type with `write_line`, would unblock real file-backed sinks.
The std::log doc page explicitly mentions "Sinks that care
about isolated streams should write to a file instead" — but
the `std::io::fs` surface that ships doesn't make that easy.

## 2026-05-10 no-eprintln-cant-isolate-debug-output

**Tried:** During debugging (before noticing the stale-CLI
issue) I wanted to print sink-internal state without
contaminating stdout, since `StdoutSink` and `println` share
the same FD.
**Hit:** No `eprintln` / `print_stderr` / process-level stderr
write primitive. The std::log doc page mentions this same gap
(WARN/ERROR routing to stderr is "a follow-up — needs an
`eprintln`-style primitive that doesn't yet exist").
**Workaround:** Tagged stdout lines with `DEBUG ` prefixes,
removed them once the bug was fixed.
**Why it matters:** Minor papercut for app-dev, but it is the
exact gap blocking the stdlib `StdoutSink` from doing
WARN/ERROR → stderr routing. Worth bundling into one m96 patch
that adds `eprintln` plus updates `StdoutSink` to use it for
levels ≥ 2.
