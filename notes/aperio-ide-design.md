# Aperio IDE — design plan

> **Status (2026-05-10):** Superseded as the primary product
> target by `notes/codebase-onboarding-design.md`. The IDE-as-
> runtime-visualizer pitch in this document remains coherent
> as a *secondary* product — you watch your *running* Aperio
> program in three lotus views, exact analogue of the
> codebase-onboarder's three views of *foreign source*. The
> two products share substrate (graphics, UI, MCP); the
> codebase-onboarder ships first because it reaches every dev
> with a codebase, not just devs already writing Aperio. The
> milestones below remain valid as a phased substrate plan;
> the runtime-instrumentation arm (`std::bus::expose`,
> `lotus.debug.*`) is now optional / later.

> *The IDE is itself an Aperio program.* This is the load-bearing
> choice. Every other decision in this doc falls out of it.

## Context

Aperio is a structured-concurrency language whose central
abstraction is the locus, and whose load-bearing property is that
*every design in the system is a lotus*: memory, lifecycle,
contracts, schedulers, bus transports, modes, perspectives, function
scope (see `docs/src/grimoire/06-the-same-shape.md`).

The thesis driving this work: **a 3D rendering of the lotus that
both human and agent can see and discuss is a feedback-loop
inflection point** — text descriptions of structure become
pointing-at-the-same-artifact; the human-agent collaboration gains
dimensional triangulation that prose alone cannot.

The IDE that does that is itself an Aperio program. Two consequences
fall out of that choice:

1. **Interop is free.** The IDE and the Aperio programs it observes
   are peer loci on the same bus. There is no custom debug-bridge
   wire format — the IDE simply `subscribe`s to `lotus.debug.*`
   subjects published by the running program. Cross-process bus
   already has a transport (`std::io::tcp` + `bus::tcp::Adapter`).
2. **Dogfooding.** Every missing primitive becomes a language
   deliverable, not a workaround. Graphics, shell-embed, MCP,
   file-watch, and self-introspection each become an `std::*`
   namespace in the m71–m93 pattern, useful to every Aperio
   program. The IDE is the forcing function for a Phase 6
   substrate roadmap.

**Compiler state since this plan's last iteration.** The
leading-edge dev session shipped m77–m93. Phases 2 (test framework,
m87+m88), 3 (HTTP server, m86), 4 (text/markdown, m91), 5
(synthesis: docs-server, m92) are sealed; m93 split the bundled
`stdlib.ap` into per-domain files in
`crates/aperio-codegen/runtime/stdlib/` (`core.ap`, `io_tcp.ap`,
`http.ap`, `text.ap`, `test.ap`). **Three open language-design
questions from this plan are resolved by that work:**

- ✅ **Function pointers** (m80 — `CodegenTy::FnPtr`).
  `ShellL { on_byte: fn(Byte) -> () }` shape works.
- ✅ **Bytes return values** (m89 — Bytes codegen + `read_bytes`).
  Serialized AST payloads are expressible.
- ✅ **Multi-accept callback pattern** (m83 — `Listener` accepts
  many `Stream`s via `on_connection: fn(Stream)`). The IDE's
  many-child supervisor can use the same pattern.

This plan covers two layers:

- **The IDE as an Aperio program** — sketched here; full `.ap`
  source committed as each substrate milestone enables it.
- **The substrate milestones (m96 onwards — Phase 6)** — concrete
  C/Rust shims exposed via the existing magic `std::*`
  path-resolver pattern.

User-confirmed v0 scope: static render + file-watch +
bridge-style observer wired but quiet. MCP transport: HTTP + SSE.

> **Multi-file module support note.** The IDE source naturally
> wants to span multiple `.ap` files. That work is being
> incorporated into a separate leading-edge dev session, not
> tracked here. This plan assumes multi-file support either arrives
> along that path before the IDE source gets large, or the IDE
> temporarily ships as a single `main.ap` until it does.

## The IDE in Aperio terms

Eleven loci, structured as a supervision tree under one root. Each
is a real Aperio locus (lifecycle, contract, bus declarations).

```
IdeRootL (main())
├── SourceWatcherL    bus: pubs ide.source.changed
├── ParserL           bus: subs ide.source.changed; pubs ide.source.parsed
├── ModelL            bus: subs ide.source.parsed; pubs ide.model.changed
├── DiagnosticsL      bus: subs ide.source.parsed, ide.runtime.event.failure
├── SceneL            bus: subs ide.model.changed, ide.view.command.*
├── CameraL           bus: subs ide.view.command.focus, ide.view.command.scrub
├── OverlayL          bus: subs ide.model.changed, ide.diagnostics.changed
├── ShellL            embeds an agent-CLI subprocess via std::shell::pty
├── McpServerL        std::mcp::serve_http; tools publish ide.view.command.*
└── DebugObserverL    bus: subs lotus.debug.* (from observed Aperio programs);
                       pubs ide.runtime.event.*
```

**Subject naming:**

- `ide.source.*` — source code lifecycle inside the IDE
- `ide.model.*` — canonical lotus model state
- `ide.view.command.*` — user/agent-driven view-control
- `ide.view.state.*` — view state notifications
- `ide.diagnostics.*` — language and runtime diagnostics
- `ide.runtime.event.*` — re-published runtime observations
- `lotus.debug.*` — **language-defined**, published by the observed
  Aperio runtime when `APERIO_DEBUG=<bus_endpoint>` is set in env.

**The interop seam.** When the user runs an Aperio program with
`APERIO_DEBUG=tcp://127.0.0.1:NNNNN` set, the runtime publishes
lifecycle/bus/closure/failure events on `lotus.debug.*` over a TCP
bus adapter. The IDE has bound the same subjects on the same TCP
endpoint via deployment config. `DebugObserverL` is an ordinary bus
subscriber. **No special wire format, no special transport, no
debug-mode codepath in the IDE.** The fact that the publisher is in
a different process is invisible at the Aperio level.

## Substrate milestones (Phase 6)

Each follows the m71–m93 pattern: C or Rust shim under
`crates/aperio-codegen/runtime/`, wired into
`lower_stdlib_path_call_expr` (and its statement sibling) in
`crates/aperio-codegen/src/codegen.rs`, exposed under a new
`std::*` namespace. The Aperio-side surface (types, helper loci)
lives in a new `.ap` file under
`crates/aperio-codegen/runtime/stdlib/` (per the m93 layout),
referenced from the `STDLIB_AP_SOURCE` `concat!()` block in
`codegen.rs`.

**Sealed:**

| #        | Name                          | Surface                                                              | What it unlocked                                                         |
| -------- | ----------------------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| **m94**  | Bus subject wildcards         | Trailing `**` on subscribe + publish; computed `<-` subjects authorized via wildcard publish | Cascading-namespace logging, sub-tree subscribers; foundation for `std::log` |
| **m95**  | `std::log`                    | `Logger`, `LogEvent`, `StdoutSink` with cascading namespace          | First Phase-6 user surface; structured logging on the bus                |

**Planned (renumbered after m94/m95 took the original slots in this plan):**

| #        | Name                          | Surface                                                              | What it unlocks                                                          |
| -------- | ----------------------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| **m96**  | Filesystem watch              | `std::fs::watch::{create, next, close}`                              | `SourceWatcherL` runs; file-watch seam closes                            |
| **m97**  | `std::bus::expose`            | Source-level "make this subject reachable on TCP"                    | Cross-process logging from `.ap` source; foundation for `lotus.debug.*`  |
| **m98**  | Runtime debug instrumentation | Runtime publishes `lotus.debug.*` when env-enabled                   | `DebugObserverL` has something to subscribe to; interop seam closes      |
| **m99**  | 3D graphics                   | `std::graphics::{scene, camera, mesh, transform, animate}`           | `SceneL` renders; `CameraL` animates; render seam closes                 |
| **m100** | UI panels                     | `std::ui::{panel, label, button, text_input, ...}`                   | `OverlayL` renders properties / filters / diagnostics                    |
| **m101** | Embedded shell                | `std::shell::{pty_spawn, pty_read, pty_write, terminal_widget}`      | `ShellL` hosts the agent CLI; shell seam closes                          |
| **m102** | MCP server                    | `std::mcp::{serve_http, tool, register_tool}`                        | `McpServerL` publishes view-commands; MCP seam closes                    |
| **m103** | Compiler self-introspection   | `std::aperio::{parse, typecheck, ast_walk}`                          | `ParserL` emits real AST/types into `ModelL`; parse seam closes          |

These milestones are **independently useful** — each lights up a
substrate every Aperio program can reach for, not just the IDE. The
big substrates (m99 graphics, m100 UI, m101 shell, m102 MCP) likely
need internal sub-milestones (e.g., m99a = window+triangle, m99b =
scene+camera) sized to fit the project's "one milestone = one
shipped real thing" cadence.

The substrate side of m99 (graphics) hosts a Bevy app embedded
under `lotus_graphics_*` Rust shims (C-runtime symbols stay
`lotus_*` per project convention), the same way m72's `lotus_tcp_*`
hosts real socket I/O. Aperio source sees only the typed
`std::graphics::*` surface; Rust/wgpu/bevy live below the path-call
boundary. m102 (MCP) embeds rmcp + axum the same way. m101 (shell)
embeds portable-pty.

m98 is the only substrate item that's a *runtime* change rather
than a *stdlib* extension: `crates/aperio-runtime` (lifecycle
dispatcher) and `crates/aperio-codegen` need to emit
`lotus.debug.*` when `APERIO_DEBUG=<endpoint>` is set in env.

## Where to start — m96: `std::fs::watch`

The smallest unit of substrate work that ships independently useful
+ unblocks the first IDE seam (file-watch). Half a day of work.
This is real substrate, not an empty stub — it follows the existing
`lotus_fs.c` pattern almost exactly.

**Aperio surface** (new file
`crates/aperio-codegen/runtime/stdlib/fs_watch.ap`):

```aperio
type WatchEvent {
    path: String;
    kind: Int;  // 0=created, 1=modified, 2=removed (until sum types)
}

// std::fs::watch::create(path)   -> Int          watcher handle; -1 on error
// std::fs::watch::next(watcher)  -> WatchEvent   blocks until next event
// std::fs::watch::close(watcher) -> Bool         true on success
```

**C substrate** (new file
`crates/aperio-codegen/runtime/lotus_fs_watch.c`):

```
int  lotus_fs_watch_create(const char* path);
WatchEvent  lotus_fs_watch_next(int watcher);   // by-value struct
bool lotus_fs_watch_close(int watcher);
```

Linux: inotify (`inotify_init1`, `inotify_add_watch`, `read`).
macOS: fsevents (deferred — Linux-only initially, matching the
project's existing platform stance). The `lotus_fs.c` wrapper
pattern is the model.

**Codegen wiring** (in `crates/aperio-codegen/src/codegen.rs`):

- Three arms added to `lower_stdlib_path_call_expr` matching
  `std::fs::watch::{create, next, close}`.
- Three libc-stub-style symbol declarations added to
  `declare_builtins` for `lotus_fs_watch_*`.
- One new `include_str!("../runtime/stdlib/fs_watch.ap")` line in
  the `STDLIB_AP_SOURCE` concat. Pure type-declaration content has
  no inter-file ordering constraints with `core.ap` / `io_tcp.ap` /
  `http.ap`.

**Build wiring**: `lotus_fs_watch.c` joins the existing C
compilation step that already handles `lotus_arena.c`,
`lotus_tcp.c`, `lotus_fs.c` — likely a one-line addition wherever
`build.rs` or the codegen-side C-bundle list lives.

**Verification** — new example
`examples/fs-watch-demo/main.ap`:

```aperio
fn main() {
    let w = std::fs::watch::create(".");
    println("watching .");
    let event = std::fs::watch::next(w);
    println("got event: ", event.path, " kind=", event.kind);
    std::fs::watch::close(w);
}
```

Run it; touch a file in cwd; expect a printed event line.
End-to-end test goes in `tests/fs_watch_demo.rs` mirroring the m76
io-demo test pattern.

**Why this first.** Smallest substrate; cleanest pattern match to
existing m74/m75 work; tests independently of any IDE work; proves
the post-m93 stdlib-extension flow is still smooth; produces
something every other Aperio program can use. m98 (`lotus.debug.*`
runtime instrumentation) is the next-best follow-up because it
validates the unique-to-IDE interop claim, but it touches
`aperio-runtime` (heavier) and lacks a non-IDE consumer (no
leverage outside this thesis).

## Sketch of the IDE `.ap` source

Illustrative — full source committed file-by-file as each substrate
milestone enables it. Once m96 + m98 land, `SourceWatcherL` and
`DebugObserverL` are buildable; the rest waits on m99+.

```aperio
type SourcePathEvent { path: String; }
type LocusNode {
    id: Int;
    name: String;
    parent_id: Int;        // -1 for root
    layer: Int;
    file: String;
    line: Int;
}

locus SourceWatcherL {
    params { root_path: String = "."; }
    bus { publish "ide.source.changed" of type SourcePathEvent; }
    run() {
        let w = std::fs::watch::create(self.root_path);
        while true {
            let event = std::fs::watch::next(w);
            "ide.source.changed" <- SourcePathEvent { path: event.path };
        }
    }
}

locus DebugObserverL {
    bus {
        subscribe "lotus.debug.locus.born"      as on_born      of type LocusBornEvent;
        subscribe "lotus.debug.locus.lifecycle" as on_lifecycle of type LifecycleEvent;
        subscribe "lotus.debug.bus.publish"     as on_publish   of type BusPublishEvent;
        subscribe "lotus.debug.closure.report"  as on_closure   of type ClosureReportEvent;
        subscribe "lotus.debug.failure"         as on_failure   of type FailureEvent;
        publish   "ide.runtime.event.born"      of type LocusBornEvent;
        // ...
    }
    fn on_born(e: LocusBornEvent) { "ide.runtime.event.born" <- e; }
    // ...
}

locus IdeRootL {
    accept(c: SourceWatcherL)  { /* register */ }
    accept(c: DebugObserverL)  { /* register */ }
    // ... (m99+ children added as substrate lands)
    run() {
        SourceWatcherL { root_path: "." };
        DebugObserverL { };
        // ...
    }
}

fn main() { IdeRootL { }; }
```

## Open language-design questions (still open)

1. **Multiple `accept` types** (per spec F.11 v0 limitation:
   single-accept-type only). `IdeRootL` accepts many distinct child
   types. m83's Listener+callback pattern (single accept type,
   callback differentiates) suggests a workable shape but doesn't
   resolve directly. Either overloaded `accept` lands, or
   `self.children` becomes a sum type, or `IdeRootL` splits into a
   chain of single-type supervisors.
2. **Sum types in payloads.** `ide.runtime.event.*` carries
   variant events. Per F.18, enum-variant patterns are deferred.
   Workaround: one subject per variant (sketch above does this).
   Not blocking.
3. **`std::aperio::parse` shape.** Either a builtin path-call into
   the existing `aperio-syntax` crate (fastest, m103's scope), or
   eventually the self-hosted compiler. Decide at m103 design
   time.

This plan **does not resolve these questions** — it identifies
them as forced by the work. Same way m72 forced bus-framing
decisions and m76 tested the no-module-system claim.

## Repo layout

```
apps/
└── aperio-ide/
    ├── README.md          # how to run; how the agent attaches via MCP
    └── src/
        └── main.ap        # IdeRootL + main() (single-file until module support)

crates/aperio-codegen/runtime/
├── lotus_arena.c          (existing, m19)
├── lotus_tcp.c            (existing, m72)
├── lotus_fs.c             (existing, m74)
├── lotus_fs_watch.c       (NEW, m96)
├── lotus_graphics.rs      (NEW, m99; Bevy host)
├── lotus_ui.rs            (NEW, m100; egui host)
├── lotus_pty.c            (NEW, m101)
├── lotus_mcp.rs           (NEW, m102; rmcp + axum host)
├── lotus_aperio_meta.rs   (NEW, m103; aperio-syntax/types reflected to .ap)
└── stdlib/                (existing layout from m93)
    ├── core.ap
    ├── io_tcp.ap
    ├── http.ap
    ├── text.ap
    ├── test.ap
    ├── fs_watch.ap        (NEW, m96)
    ├── graphics.ap        (NEW, m99)
    ├── ui.ap              (NEW, m100)
    ├── shell.ap           (NEW, m101)
    ├── mcp.ap             (NEW, m102)
    └── aperio.ap          (NEW, m103)

crates/aperio-runtime/
└── (modified, m98: lifecycle dispatcher emits lotus.debug.* when
                    APERIO_DEBUG=<endpoint> is set)

crates/aperio-codegen/src/codegen.rs
└── (modified, each milestone:
    - new arm in lower_stdlib_path_call_expr
    - new include_str!() entry in STDLIB_AP_SOURCE concat
    - libc backers in declare_builtins)

docs/src/std/roadmap.md
└── (modified: Phase 6 (m96 onwards) added with the IDE as the
              named forcing function)
```

The repo dir for the IDE source is `apps/aperio-ide/` — peer to
`examples/`, marked as a real production-shape Aperio program (the
project's own primary use case). `examples/` stays pedagogical.

## MCP tool surface (v0 — 10 tools, read-mostly)

Implemented in `McpServerL` once m102 lands.

```
list_loci()                     -> [{id, name, parent_id, layer, file, span}]
get_locus(id)                   -> {params, contract, bus_subjects, closures, lifecycle_state}
focus_locus(id, transition_ms)  -> ok
highlight_subject(subject)      -> ok
set_filter({by_layer?, by_projection_class?, by_closure_status?}) -> ok
get_view_state()                -> {focused_id, camera_pose, filter, time_cursor}
list_subjects()                 -> [{subject, publishers: [id], subscribers: [id]}]
get_diagnostics()               -> [{path, span, severity, message}]
scrub_time(t_ns | "live")       -> ok
open_source(path, span?)        -> ok
```

Discipline: every tool either reads view+model state or publishes
exactly one `ide.view.command.*`. **No tool mutates `LotusModel`
directly** — model changes always come from source edits. The
agent can drive what the human sees; it cannot silently rewrite
the world. `edit_source` deferred until v0 is observed in real use.

## Verification — six seams

The seams correspond to substrate milestones. Full demo
(target: post-m103) runs against
`/home/riley/code/lotus-lang/examples/02-parent-child/main.ap`:

1. **File-watch seam (post-m96).** Edit and save the example. The
   IDE's `SourceWatcherL` emits `ide.source.changed` within 300ms.
2. **Interop seam (post-m98).** Run the example with
   `APERIO_DEBUG=tcp://127.0.0.1:7778` and bind the IDE's
   `DebugObserverL` to the same endpoint. Lifecycle events flow.
3. **Render seam (post-m99).** `SceneL` renders the parsed
   structural lotus: `main()`, `CoordinatorL`, three `GreeterL`
   siblings.
4. **UI seam (post-m100).** `OverlayL` shows the focused locus's
   properties + a (empty) diagnostics panel.
5. **Shell seam (post-m101).** `ShellL` hosts `claude` in an
   embedded pane.
6. **MCP seam (post-m102).** External agent connects to
   `127.0.0.1:7777`, calls `list_loci()` → 4 entries, calls
   `focus_locus(coordinator_id, 500)` → `CameraL` animates over
   500ms.

If any seam fails, that substrate milestone is the only thing to
fix. Nothing downstream of it matters yet.

## Pre-commit spike (before the big substrate work)

After m96 + m98 ship, but before committing to m99–m103 (the heavy
substrate work): two-day spike implementing **just enough
`lotus_graphics_*`** (a minimal Bevy host) AND a hand-written
`.ap` IDE-stub that calls `std::graphics::*` to draw
`examples/02-parent-child`'s structural lotus. Goal: confirm "the
geometry of the tree is the right primary surface."

If after two days the result feels like an IDE worth using, commit
to m99–m103. If not, the entire hypothesis is wrong; no substrate
roadmap will save it.

## Risks and kill-switches

| Risk                                                                            | Detect-point                                                | Pivot                                                                                                                  |
| ------------------------------------------------------------------------------- | ----------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| Pre-commit graphics spike doesn't feel like an IDE worth using                   | End of day 2                                                | Stop. The thesis is wrong; reconsider 2D-tree-with-syntax-highlight.                                                    |
| `lotus.debug.*` runtime instrumentation costs too much                          | m98 perf measurement                                        | Gate harder behind env var; default-off                                                                                  |
| Bevy embedded under `lotus_graphics_*` is awkward to expose typed-Aperio-side    | m99 surface design                                          | Try `three-d` or hand-rolled wgpu; the substrate boundary moves, the .ap source is unchanged                            |
| `rmcp` HTTP+SSE flaky under m102                                                  | claude-cli connect tests                                    | Stdio transport instead; agent runs as IDE-spawned subprocess via m101                                                    |
| `accept` of multiple types blocks `IdeRootL`                                     | when wiring is attempted (around m103)                     | Either overloaded-accept lands as a language deliverable, or `IdeRootL` is restructured as a chain of single-type supervisors |
| Substrate ships but no Aperio program ever consumes it except the IDE            | m99+ adoption observation                                   | Acceptable — the IDE is the forcing function; secondary consumers come later                                              |

## Critical files

**To create (m96 — start here):**

- `crates/aperio-codegen/runtime/lotus_fs_watch.c`
- `crates/aperio-codegen/runtime/stdlib/fs_watch.ap`
- `examples/fs-watch-demo/main.ap`
- `examples/fs-watch-demo/README.md`
- `tests/fs_watch_demo.rs` (or equivalent, mirroring m76 io-demo
  test pattern)

**To create (later milestones):**

- `crates/aperio-codegen/runtime/lotus_graphics.rs` (m99)
- `crates/aperio-codegen/runtime/lotus_ui.rs` (m100)
- `crates/aperio-codegen/runtime/lotus_pty.c` (m101)
- `crates/aperio-codegen/runtime/lotus_mcp.rs` (m102)
- `crates/aperio-codegen/runtime/lotus_aperio_meta.rs` (m103)
- `crates/aperio-codegen/runtime/stdlib/{graphics,ui,shell,mcp,aperio}.ap`
- `apps/aperio-ide/src/main.ap`
- `apps/aperio-ide/README.md`

**To modify:**

- `crates/aperio-codegen/src/codegen.rs` — for each milestone, add
  arms to `lower_stdlib_path_call_expr`, libc backers in
  `declare_builtins`, and one `include_str!()` line in
  `STDLIB_AP_SOURCE` concat.
- `crates/aperio-runtime/src/bus.rs`, `lifecycle.rs` (or wherever
  the dispatcher lives) — m98 `lotus.debug.*` emission gated by
  env.
- C-bundle build wiring (likely `crates/aperio-codegen/build.rs`
  or similar) — add new `lotus_*.c` files.
- `Cargo.toml` — workspace member additions if any new crates are
  introduced (default: stay in `aperio-codegen/runtime/` per
  existing pattern).
- `docs/src/std/roadmap.md` — extend Phase plan to include Phase 6
  (m96 onwards) with the IDE as forcing function.

**To reuse (existing, no changes):**

- `crates/aperio-syntax/src/ast.rs` — read by m103's
  `lotus_aperio_meta.rs` substrate to expose AST to `.ap` source.
- `crates/aperio-types/src/lib.rs` — same.
- m71's magic `std::*` path resolver in `aperio-codegen` — every
  new namespace added here.
- m72's `lotus_tcp_*` framing pattern — m98's `lotus.debug.*`
  transport reuses it.
- m26's bus router and m28b's mailbox infrastructure — m98's
  lifecycle-event emission slots in here.
- m74's `lotus_fs.c` pattern — m96's `lotus_fs_watch.c` is a near
  twin.

## Sequence

1. **m96** — `std::fs::watch`. Walkthrough above. ~half a day.
   Validates the post-m93 substrate-add pattern.
2. **m98** — runtime `lotus.debug.*` emission. Touches lifecycle
   dispatcher; gated by env. Uses m72 TCP framing for cross-process.
3. **(spike, 2d)** — minimal `lotus_graphics_*` rendering
   `02-parent-child`. Go/no-go on the geometry thesis.
4. **m99** — `std::graphics`. Big; likely sub-milestones m96a,
   m96b, m96c.
5. **m100** — `std::ui`. egui host.
6. **m101** — `std::shell`. portable-pty host.
7. **m102** — `std::mcp`. rmcp + axum host.
8. **m103** — `std::aperio::parse` / `typecheck`. Reflects compiler
   crates.
9. **v0 tag** — six-seam demo passes.

The substrate is the slow part. Each milestone is independently
useful to other Aperio programs — `std::fs::watch`,
`std::graphics`, `std::ui`, `std::shell`, `std::mcp`,
`std::aperio` all stand on their own. The IDE just happens to be
their first big consumer.
