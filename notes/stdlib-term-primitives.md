# `std::term` + raw byte I/O primitives (pond P4)

Status: **scope / proposal.** Nothing built. Written 2026-06-09 from the
pond terminal-stack handoff (P4, `term/FRICTION.md § stdlib-term-primitives`).
Five libc one-liners pond ships as FFI glue are generic OS surface, not
terminal-styling logic — they want a `std::` home so a color-aware logger
or a TUI doesn't have to vendor an FFI lib (with the C-symbol-collision +
duplicate-glue tax that drags in, see P5).

## Motivation (the concrete unblock)

`pond/logfmt`'s `ConsoleSink` can't answer *"is stderr a tty?"* without
vendoring an FFI shim, so it takes a `color: Bool` param and defaults
`true` + honors `NO_COLOR`. With `std::term::is_tty` it would just probe.
Same story for any program that wants terminal size or raw input without
an FFI dependency.

## The five primitives (pond's reference impls are the basis)

pond/term/glue.c already has clean, generic implementations — this scope
is mostly about *where they live* and *what shape they return* once
they're stdlib, not new logic:

| pond shim | what it does | proposed stdlib surface |
|---|---|---|
| `term_isatty(fd)` | `isatty(fd)` | `std::term::is_tty(fd: Int) -> Bool` |
| `term_size_packed()` | `ioctl(TIOCGWINSZ)`, returns `(cols<<16)\|rows` | `std::term::size() -> TermSize fallible(IoError)` |
| `term_raw_enable/disable()` | `tcgetattr`/`tcsetattr` raw toggle + atexit restore | `std::term::RawMode` guard locus (below) |
| `term_write_stdout(s)` | raw `write(1, ...)`, bypass `_IOLBF` | `std::io::stdout::write_bytes(s) -> Int fallible(IoError)` |
| `term_read_byte(timeout_ms)` | `poll` + 1-byte `read` | `std::io::stdin::read_byte(timeout_ms: Int) -> Int` |

The C bodies move verbatim into the lotus runtime (a new `lotus_term.c`,
or appended to `lotus_arena.c`) as `lotus_term_*`, wired by path-call
dispatch + a builtins declaration — the same pattern as
`std::process::rss_bytes` / `std::io::stdin::read_line` (which already
exists, so `read_byte` slots beside it).

## Module placement

- **`std::term`** — terminal-specific: `is_tty`, `size`, `RawMode`.
- **`std::io::stdout` / `std::io::stdin`** — generic raw byte I/O:
  `write_bytes`, `read_byte`. (`std::io::stdin::read_line` is already
  there; this rounds out the byte-level surface.) `is_tty` takes any fd
  but reads as terminal-capability probing, so it sits in `std::term`.

## Shape decisions (stdlib-ifying pond's Int hacks)

pond packs everything into `Int` because that's all FFI gives it cheaply.
Stdlib should return real shapes:

1. **`size() -> TermSize fallible(IoError)`** — a record
   `type TermSize { cols: Int; rows: Int }` rather than pond's
   `(cols<<16)|rows` bit-pack. Not-a-tty / ioctl-fail → `IoError` (the
   established fallible error), not a magic `0`. Callers that poll per
   frame `or` a default.
2. **`write_bytes(s) -> Int fallible(IoError)`** — bytes written; `-1`
   becomes an `IoError`. Matches `std::io::fs::*` fallible convention.
   **Must `fflush(stdout)` first** (see buffering, below).
3. **`read_byte(timeout_ms) -> Int`** — keep the low-level **sentinel**
   return (`0..255` = the byte, `-1` = timeout, `-2` = EOF/error), *not*
   fallible: a timeout is an ordinary control outcome on a poll loop, not
   an error, and a sum-typed return is heavier than this hot path wants.
   Document the sentinels. (A higher-level `read_key` that decodes escape
   sequences into a key enum is a library, not a primitive — out of scope.)
4. **`is_tty(fd) -> Bool`** — straight `Bool`, not `0/1`.

## The big idea: `RawMode` as a guard locus + a runtime atexit backstop

pond's raw-mode toggle pair plus an atexit restore is exactly the RAII
shape Hale models with a **guard locus** (like `BytesBuilder`):

```hale
fn main() {
    let raw = std::term::RawMode { };   // birth() -> tcsetattr raw
    // ... interactive loop ...
}                                       // dissolve() -> restore termios
```

`birth()` enters raw mode, `dissolve()` restores it at scope exit — no
manual `disable()` to forget. **And the runtime's raw-enable primitive
should register the termios restore via `atexit` itself** (idempotent,
like pond's glue). That composes with **P2** (`#106`: panics now exit via
`exit()`, atexit-visible): a stale-view panic, an unhandled error escaping
`main`, or a normal return all restore the terminal — *with no FFI glue at
all*. This retires pond/term's hand-rolled atexit restore entirely, which
is the real prize here: terminal hygiene becomes the runtime's job.

## Buffering interaction (the `_IOLBF` gotcha)

The prelude does `setvbuf(stdout, NULL, _IOLBF, 0)` (`lotus_arena.c:8815`)
so `\n`-terminated `println` flushes per line. `write_bytes` does a raw
`write(2)` that bypasses that buffer — so a frame written via `write_bytes`
after some `println` output would be **reordered** (the buffered text
flushes later). `std::io::stdout::write_bytes` must therefore
`fflush(stdout)` before its raw write, so the two ordering domains stay
consistent. Document that interleaving `println` and `write_bytes` is
otherwise the caller's hazard.

## Portability

All five are POSIX (`termios` / `ioctl(TIOCGWINSZ)` / `poll`). Non-tty
fds degrade gracefully (`is_tty` false, `size` errors, `RawMode.birth`
fails-soft → the program runs unstyled). Windows is unsupported at this
layer (a separate console-API backend would be its own effort); guard the
runtime with `#if defined(__unix__) || defined(__APPLE__)` and have the
primitives return the not-a-tty results on other platforms so builds don't
break.

## Out of scope / future

- **Parkable stdin on `where async_io` pools.** The handoff's longer-term
  ask: an interactive app should *park* on fd 0 instead of poll-sleeping
  via `read_byte(timeout)`. F.35 parks sockets, not fd 0 — extending the
  pool's epoll set to stdin is a real runtime change, separate from these
  primitives. `read_byte(timeout)` is the poll-based stopgap until then.
- **`read_key` / escape-sequence decoding, color/style helpers, a cell
  grid** — all library territory on top of these primitives (pond/tui
  *is* that library); the stdlib ships the OS surface, not the TUI.

## Staging

1. **`std::term::is_tty` + `std::io::stdout::write_bytes`** — the two
   `ConsoleSink` actually needs, smallest surface. `lotus_term_is_tty` +
   `lotus_term_write_stdout` (with the `fflush`), path-call dispatch,
   builtins decls. Unblocks pond/logfmt immediately.
2. **`std::term::size`** + the `TermSize` record.
3. **`std::term::RawMode` guard locus + the runtime atexit backstop** —
   the piece that composes with P2 and retires pond's glue.
4. **`std::io::stdin::read_byte(timeout)`** — beside `read_line`.
5. Update `spec/stdlib.md` (the `std::term` + `std::io` rows) and
   `spec/ffi.md` (note that these no longer need vendored glue) as each
   lands.

## Risks

- **Surface creep into a TUI.** Stop at the five OS primitives + `RawMode`.
  Key decoding, styling, and grids are pond/tui's job; pulling them into
  std would bloat it and freeze a TUI model prematurely.
- **The `write_bytes` / `println` ordering** is a real footgun; the
  mandatory `fflush` + a spec note are the mitigation, but mixed use stays
  caller-beware.
- **Raw-mode restore on a hard crash** (SIGKILL, SIGSEGV without a
  handler) still strands the terminal — atexit doesn't run on an
  uncatchable signal. P2 covers the panic/`exit` paths; a SIGSEGV handler
  that restores + re-raises is a possible future hardening, noted not done.
