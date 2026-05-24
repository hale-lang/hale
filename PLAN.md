# iris — buildout plan

This document is the implementation roadmap for iris. The
*strategic* frame lives in [`VISION.md`](./VISION.md); the
*contracts* live in [`AGENT_PROTOCOL.md`](./AGENT_PROTOCOL.md)
(MCP read tools + write-proposal schema),
[`COMPILER_FFI.md`](./COMPILER_FFI.md) (binding surface for
raylib/PTY/SSE), and [`PLUGINS.md`](./PLUGINS.md) (future
plugin surface — currently deferred). This file says *what to
build and in what order*.

Read VISION.md first; it sets the frame everything below
derives from.

---

## 1. What iris is (one-paragraph summary)

iris is a **modeling cockpit for structured information** —
not an IDE. A **trinity** of always-visible panes (viz,
source, agent) hosts the modeling loop: viz renders the
locus structure as a lotus flower, source carries the model
text, agent steers. A fourth pane (shell) lives as a
toggleable drawer beneath the viz pane — present and
mechanically equivalent but visually summoned because it's
an escape hatch, not part of the loop. For `.hl` projects
the codebase IS the model; for non-Hale domains the agent
writes Hale on the fly that models the domain and the
flower renders that. One notation (Hale), one default
visualization (the flower), every domain. External agents
(Claude Code, Cursor, …) summon iris via MCP for tasks that
fit; the developer's primary editor stays. See VISION.md §1
+ §4 + §13 for the full framing.

## 2. First principles

Non-negotiable. Every implementation decision checks against
them; deviations need a `FRICTION.md` entry.

1. **Modeling, not editing.** iris's value is concurrent
   human+agent work on locus structure. Source editing is
   one realization. If a feature makes iris better at being
   a primary editor at the cost of being worse at being a
   structural cockpit, the feature is rejected.

2. **Two channels, never one.** User input and programmatic
   commands are separate bus subject families. A pane
   subscribes to both; neither family knows the other exists.
   This is what enables concurrent human+agent work without
   contention.

3. **Read/write split.** Agent **reads** are in-band MCP
   tools (idempotent, side-effect-free; nondeterminism
   contained to "did the agent decide to look"). Agent
   **writes** are out-of-band structured-output proposals
   parsed from response text and dispatched by a mechanical
   outer loop (nondeterminism contained to "did the agent
   decide to propose"; dispatch from a proposal is
   deterministic). See AGENT_PROTOCOL.md for the contract.

4. **PermissionGate is structural enforcement.** Lives in the
   dispatch path — `ProposalDispatcher.on_proposal` consults
   the gate before republishing to `ide.cmd.<pane>.*`. Not a
   convention, not a popup, not advisory. The gate's state is
   the contract; trust is mechanical.

5. **iris is summoned, not switched-to.** Distribution
   through external-agent MCP discovery, not VSCode
   competition. iris ships as a binary on PATH;
   `iris --help` is rich and agent-readable; external agents
   call `iris.launch(workspace=...)` when the task fits.

6. **Every named structural thing is a locus.** Panes, tools,
   harness, renderer, gate, MCP server, proposal extractor,
   proposal dispatcher, session store — all loci. F.19
   per-directory seed; F.29 cascade for lifecycle.

7. **Cooperative scheduling, no threads.** Bus delivery is
   synchronous in the publish-call stack (confirmed in
   `FRICTION.md` "useful findings"); long-running work yields
   between iterations.

8. **Vertical-only flow.** Cross-pane state moves through
   bus subjects. No pane reads another pane's mutable state
   via locus references. (PermissionGate is the one
   exception: queried by ProposalDispatcher, never mutated
   cross-locus.)

## 3. Architecture

### 3.1 Locus tower

```
Iris                            # app locus, root
├── UserConfig                  # ~/.config/iris/config.json
├── WorkspaceConfig             # <workspace>/.iris/config.json
├── SessionStore                # <workspace>/.iris/session.json
├── PermissionGate              # per-pane (read, write) policy
├── McpServer                   # JSON-RPC server hosting read tools
├── ProposalExtractor           # parses iris.proposal{} blocks from chunks
├── ProposalDispatcher          # gates + dispatches proposals to ide.cmd.*
├── Renderer                    # window + frame loop (raylib-backed)
├── Layout                      # trinity tiling + shell drawer state
├── InputCapture                # window-system events → ide.input.*
├── AgentHarness                # active AgentBackend host
├── VizPane                     # trinity — flower scene host (left ~2/3, full-height by default)
├── SourcePane                  # trinity — readout (right column top)
├── AgentPane                   # trinity — streaming prompt + response (right column bottom)
└── ShellPane                   # drawer — PTY subprocess + scrollback (collapsed by default; toggled via Ctrl+`)
```

Every locus listed above is held as an F.29 field on its
parent. Lifecycle cascades down on dissolve; no manual
teardown.

**Pane visibility status.** Trinity panes (Viz / Source /
Agent) are always-visible per the cockpit commitment
(VISION §13). ShellPane is mechanically equivalent — same
bus channels, same PermissionGate enforcement, same
proposal dispatch surface — but its render is gated by
Layout.shell_drawer_open. When closed, ShellPane still
exists, still receives bus events on `ide.cmd.shell.*`,
still has scrollback accumulating in its PTY buffer; it's
just not rendered to screen until the user toggles the
drawer.

### 3.2 The `Pane` interface (F.20)

```hale
interface Pane {
    fn focus_gain();
    fn focus_lose();
    fn render(s: RenderSurface);
    fn name() -> String;
    fn on_key(k: KeyEvent);     // un-handled keys when focused
}
```

A `@form(vec)<Pane>` on `Iris` aliases the four pane fields
for uniform iteration (Renderer + focus dispatch). The vec
doesn't own; the fields do.

### 3.3 Bus subject schema

**Input channel** (raw + focus-routed user input):

| Subject | Publisher | Subscriber | Payload |
|---|---|---|---|
| `ide.input.key` | InputCapture | Iris | `KeyEvent` |
| `ide.input.<pane>.key` | Iris (routed) | Pane | `KeyEvent` |
| `ide.input.focus_request` | InputCapture, panes | Iris | `FocusRequest` |

**Command channel** (programmatic, always-on):

| Subject | Publisher | Subscriber | Payload |
|---|---|---|---|
| `ide.cmd.source.open` | ProposalDispatcher | SourcePane | `SourceOpen` |
| `ide.cmd.source.goto` | ProposalDispatcher | SourcePane | `SourceGoto` |
| `ide.cmd.source.write` | ProposalDispatcher | SourcePane | `SourceWrite` |
| `ide.cmd.shell.exec` | ProposalDispatcher | ShellPane | `ShellExec` |
| `ide.cmd.viz.add_entity` | ProposalDispatcher / VizPane | VizPane | `EntityAdd` |
| `ide.cmd.viz.camera` | ProposalDispatcher | VizPane | `CameraSet` |
| `ide.cmd.viz.mode` | ProposalDispatcher | VizPane | `ModeSet` |
| `ide.cmd.viz.focus` | ProposalDispatcher | VizPane | `EntityFocus` |
| `ide.cmd.gate.set` | UI keybind handler (NOT agent) | PermissionGate | `GateSet` |

**Proposal channel** (agent writes — see AGENT_PROTOCOL.md §2):

| Subject | Publisher | Subscriber | Payload |
|---|---|---|---|
| `ide.proposal.in` | ProposalExtractor | ProposalDispatcher | `Proposal` |
| `ide.proposal.applied` | ProposalDispatcher | AgentPane, audit | `Applied` |
| `ide.proposal.denied` | ProposalDispatcher | toast UI, audit | `Denial` |

**Agent flow** (between AgentPane / Harness / Backend):

| Subject | Publisher | Subscriber | Payload |
|---|---|---|---|
| `ide.agent.user_input` | AgentPane | AgentHarness | `UserPrompt` |
| `ide.event.agent.chunk` | active AgentBackend | AgentPane, ProposalExtractor | `Chunk` |
| `ide.event.agent.turn_complete` | active AgentBackend | AgentPane, ProposalExtractor | `TurnComplete` |
| `ide.event.agent.error` | active AgentBackend | AgentPane, toast UI | `AgentErrorPayload` |

**Event channel** (state changes broadcast):

| Subject | Publisher | Subscriber | Payload |
|---|---|---|---|
| `ide.event.focus_changed` | Iris | all panes | `FocusChange` |
| `ide.event.source.changed` | SourcePane | SessionStore, VizPane (extraction) | `SourceChanged` |
| `ide.event.viz.changed` | VizPane | SessionStore | `VizChanged` |
| `ide.event.gate.changed` | PermissionGate | pane border UI | `GateChange` |

The schema is closed. Adding a subject is a deliberate design
move that updates this table first.

### 3.4 Agent interaction surface

Detailed in [`AGENT_PROTOCOL.md`](./AGENT_PROTOCOL.md).
Summary:

- **MCP read tools** — external agents discover and call via
  the standard MCP protocol. `iris.launch`, `pane.read`,
  `pane.focus_state`, `viz.snapshot`, `viz.find`,
  `bus.recent`, `permission.state`. Hosted by `McpServer`.
- **Write proposals** — agent emits
  `iris.proposal { channel, payload, reason }` blocks in
  response text. `ProposalExtractor` parses the chunk
  stream; `ProposalDispatcher` validates against
  `PermissionGate` and dispatches to `ide.cmd.<pane>.*`.

The same proposal protocol applies to iris's *internal* agent
(via `AgentHarness` + the active `AgentBackend`) and to
*external* agents (over MCP). Backends differ; the
read/write split is universal.

### 3.5 Pluggable interfaces

Three interfaces are load-bearing today — implementations
exist or are planned. The plugin *manifest* (build-time
registry) is deferred (§3.6) but the interfaces themselves
are real.

#### `AgentBackend`

```hale
interface AgentBackend {
    fn name() -> String;
    fn model() -> String;
    fn is_ready() -> Bool;
    fn load_history();
    fn submit_user_input(prompt: String);
    fn cancel();
}
```

Note the change from earlier sketches: **no
`submit_tool_result`**. Tools-as-callbacks are gone with the
read/write split. Reads are out-of-band MCP (the backend
doesn't relay them); writes are proposals in the response
stream (the backend just streams text and let
ProposalExtractor handle the rest).

First reference backend: `EchoBackend` (deterministic, in
spike code). Second: `AnthropicBackend` (HTTP+SSE; Phase 4).
Third: `Brain3Backend` (HTTP via brain3 openapi; later phase).

#### `VizScene`

The viz pane hosts a pluggable scene. iris's substrate is
generic (3D primitives, camera, modes); the *content* of the
viz is scene-defined. Each scene declares what to render,
what data feeds it, and what API the agent can use to drive
it. Different domains own their own viz.

```hale
interface VizScene {
    fn name() -> String;        // "hale.locus_tower", "brain3.topology", ...
    fn domain() -> String;      // "hale", "brain3", "<demo-app>", ...
    fn install();               // becomes the active scene
    fn uninstall();             // replaced or VizPane dissolves
    fn render(s: RenderSurface);
    fn camera() -> Camera3D;    // substrate concern, every scene has one
    fn set_camera(c: Camera3D);
    fn channels() -> [SceneChannelSpec];   // agent API discovery
    fn dispatch(channel: String, payload_json: String);
    fn on_source_changed(file: String, content: String);   // optional reactivity
    fn on_click(screen_x: Int, screen_y: Int);             // user clicked in pane
}

type SceneChannelSpec {
    channel:        String;     // e.g. "viz.scene.add_node"
    payload_schema: String;     // JSON-Schema describing the payload
    description:    String;
}
```

VizPane holds exactly one active scene at a time. Switching
scenes is config or proposal-driven (`viz.set_scene`
proposal). Agent discovery of what the active scene exposes
is via the MCP read tool `viz.scene_info()` (see
AGENT_PROTOCOL.md).

`on_click` is delegated end-to-end to the scene: VizPane
forwards pane-relative screen coordinates; the scene
unprojects against its own camera, hit-tests against its own
node positions, and decides what to do (emit a proposal,
update camera focus, no-op, …). Click semantics are
domain-specific by definition — clicking a brain3 task node
means something different from clicking an Hale locus.

**Default scene: `lotus.flower`.** Universal renderer for any
Hale locus tower. Parses `.hl` source from the workspace +
the agent's in-session writes (`<workspace>/.iris/models/*.hl`)
and renders the tower as concentric petals radiating from the
root. Projection class drives color family (rich / chunked /
recognition); lifecycle drives animation (birth → grow,
mutation → shimmer, dissolve → wilt); bus traffic pulses
edges. Scene channels: `viz.scene.spotlight(ids)`,
`viz.scene.pulse(id, color)`, `viz.scene.draw_edge(a, b)`,
`viz.scene.narrate(id, text)` — the agent's cinematic
controls.

The flower works in two cases without per-domain code:

- **Hale projects:** flower renders the codebase's actual
  locus tower. iris is an IDE out of the box.
- **Non-Hale domains:** the agent writes Hale that models
  the domain (`org.hl`, `strategy.hl`, `claims.hl`, …);
  flower renders the agent's model. The user reviews the
  `.hl` in the source pane. Persisted in
  `<workspace>/.iris/models/`.

This makes the flower the *universal floor*. Every domain
works decently without any plugin because Hale's
substrate-invariance means every domain can be modeled in
Hale; the flower can render any Hale locus tower.

**Domain specialization is plugin territory, three shapes:**

1. **Replace.** Ship a scene that takes over the viz pane.
   Right for domains with strong non-flower visual
   conventions (brain3's directed-DAG topology, <demo-app>'s
   strategy-tower-with-PnL overlay).
2. **Decorate.** Attach domain-specific glyphs / colors /
   labels / animations to flower petals via scene-channel
   proposals. Same flower, richer per-petal info.
3. **Layer.** Flower as substrate, domain overlay on top —
   probably needs a `fn overlay_render(s)` interface
   addition; deferred until a workload demands.

Plugin scenes (`Brain3TopologyScene`, `<demo-app>StrategyScene`,
`ClaimGraphScene`, …) ship with their respective backends or
as standalone plugins. Each picks one of the three shapes.

#### `Pane`

(Already in §3.2.) Listed here for completeness — pluggable
in principle; the four built-ins are wired today. Custom
panes wait for a workload that demands one.

### 3.6 Plugin surface — deferred

PLUGINS.md sketches a build-time manifest + generated
registry for plugin selection across `AgentBackend`,
`VizScene`, `Pane`. **Marked deferred per VISION §13** —
the interfaces are load-bearing; the manifest / generated
registry / config-driven selection wait for a workload that
needs them. At v0 the active backend and active scene are
hardcoded in `AgentHarness` and `VizPane` respectively.

### 3.7 PermissionGate (structural enforcement)

```hale
type PanePolicy { read: Bool = true; write: Bool = true; }

locus PermissionGate {
    params {
        shell:  PanePolicy = PanePolicy { };
        source: PanePolicy = PanePolicy { };
        viz:    PanePolicy = PanePolicy { };
        agent:  PanePolicy = PanePolicy { };
        audit:  AuditLog;
    }
    fn allows(pane: String, op: String) -> Bool { ... }
    fn deny(pane: String, op: String, why: String) { ... }
}
```

The gate is queried by `ProposalDispatcher` before every
proposal dispatch. There is no other path to mutate iris
state (modulo direct user input through `ide.input.<pane>.*`);
the gate is the single enforcement point. UI keybinds
(Ctrl+L) publish `ide.cmd.gate.set` to update policy; the
agent can never publish to that subject (denied by the
ProposalDispatcher's channel-allowlist).

### 3.8 Configuration

```
~/.config/iris/config.json        # user-global
<workspace>/.iris/config.json     # workspace-local (overrides user)
```

Initial schema:

```json
{
  "keybinds": {
    "global": [
      {"key": "Ctrl+Tab", "cmd": "cycle_focus_forward"},
      {"key": "Ctrl+Shift+Tab", "cmd": "cycle_focus_backward"},
      {"key": "Ctrl+1", "cmd": "focus_pane_shell"},
      {"key": "Ctrl+L", "cmd": "toggle_pane_lock"},
      {"key": "Ctrl+Q", "cmd": "quit"}
    ]
  },
  "theme": { "background": "#1a1a1a", "foreground": "#eaeaea" },
  "layout": { "default": "tiled-2x2" },
  "agent": {
    "backend": "echo",
    "backend_config": {
      "anthropic": { "model": "claude-opus-4-7", "api_key_env": "ANTHROPIC_API_KEY" },
      "brain3":    { "api_url": "...", "project_id": "...", "node_id": "..." }
    }
  },
  "mcp": {
    "transport": "stdio",
    "listen_socket": null
  },
  "session": { "auto_restore": true, "save_debounce_ms": 5000 }
}
```

### 3.9 Workspace state

`<workspace>/.iris/`:

```
.iris/
├── config.json          # workspace-local config (committed)
├── session.json         # last layout, open files, cursor positions
├── permissions.json     # PermissionGate state
├── proposals.log        # audit log of every dispatched / denied proposal
└── agent_history.db     # backend-managed conversation persistence
```

`proposals.log` is new (vs. spike); it's the dispatch audit
trail VISION §3 implies ("Every gate transition is a bus
event. The history of who-could-do-what-when is the bus
log."). One line per `ide.proposal.applied` and
`ide.proposal.denied` event.

### 3.10 File layout

```
iris/
├── VISION.md                   # strategic frame (read first)
├── PLAN.md                     # this file
├── README.md                   # orientation + usage
├── AGENT_PROTOCOL.md           # MCP read tools + write proposal contract
├── COMPILER_FFI.md             # binding surface contract
├── BINDINGS_HANDOFF.md         # how-to-pick-up brief for pond bindings
├── PLUGINS.md                  # deferred future plugin surface
├── FRICTION.md                 # design / language friction log
├── KNOWN_GOTCHAS.md            # implementation surprises
├── main.hl                     # Iris locus + fn main()
├── messages.hl                 # shape types
├── topics.hl                   # bus topic decls
├── pane.hl                     # Pane interface + PaneList
├── {shell,source,viz,agent}_pane.hl
├── viz_scene.hl                # VizScene interface + SceneChannelSpec (Phase 3)
├── file_watcher.hl             # fsnotify wrapper driving SourcePane readout
├── layout.hl                   # Layout
├── input_capture.hl            # InputCapture
├── renderer.hl                 # Renderer + frame loop
├── permission_gate.hl          # PermissionGate + types
├── mcp_server.hl               # McpServer (Phase 2)
├── proposal_extractor.hl       # ProposalExtractor (Phase 1)
├── proposal_dispatcher.hl      # ProposalDispatcher (Phase 1)
├── agent_backend.hl            # AgentBackend interface
├── agent_backend_echo.hl       # EchoBackend
├── agent_harness.hl            # AgentHarness
├── config.hl                   # config loaders (Phase 7)
├── session.hl                  # SessionStore (Phase 7)
├── cli.hl                      # CLI parsing (Phase 7)
├── lib/                        # locally-maintained libs
│   ├── raylib/                 # @ffi-backed real raylib bindings (Phase 5.a)
│   ├── pty/                    # forkpty stub → pond/pty later
│   ├── sse/                    # SSE stub → real after std::http::recv_chunk
│   ├── http/                   # pond/http snapshot (Phase 4)
│   ├── sqlite/                 # pond/sqlite snapshot (later)
│   ├── heron/                  # tree-sitter Hale grammar + @ffi wrapper (snapshot of pond/heron)
│   ├── lotus_viz/              # LotusFlowerScene + supporting model/layout/animator/aesthetics
│   └── _util/                  # ad hoc helpers
└── tests/
    └── stub_smoke/             # FFI surface smoke test
```

All top-level `.hl` files form one seed (F.19). Each
directory under `lib/` is its own F.19 sub-seed, consumed
via `import "lib/<name>" as <alias>;`.

## 4. Substrate prerequisites

Things that must land before / alongside iris.

**Strategy: local stubs today, pond bindings via `@ffi`
later.** Detail in [`COMPILER_FFI.md`](./COMPILER_FFI.md);
hale is landing `@ffi("c") fn …;` + `hale.toml [ffi]`
as a one-time compiler change. After Stage 1, raylib + PTY
bindings land as ordinary pond lib work in `pond/raylib` and
`pond/pty` — no further compiler involvement.

| Need | Choice | Status |
|---|---|---|
| Window + 3D + input + text | raylib | `pond/raylib` after `@ffi` Stage 1; stubbed at `iris/lib/raylib/` today |
| PTY | `forkpty` + fd I/O | `pond/pty` after `@ffi` Stage 1; stubbed at `iris/lib/pty/` today |
| SSE parsing | pure Hale over `std::http::*` | Stub at `iris/lib/sse/` today; promote to real once `std::http::recv_chunk` confirmed |
| HTTP client | snapshot of `pond/http/client` | Not yet vendored; lands in Phase 4 |
| MCP JSON-RPC | pure Hale over stdio | No prereq; lands in Phase 2 |
| AST extraction for `.hl` source | `pond/heron` (tree-sitter grammar + @ffi wrapper) | Shipped. Vendored at `iris/lib/heron/`. 23/23 corpus tests; 19/19 stdlib + 33/33 iris files parse cleanly. Requires `libtree-sitter-dev` at build time. |
| File watching (fsnotify) | inotify on Linux | Source pane is readout-only (§3.4); fsnotify drives the re-display loop. If `std::io::fs::watch` doesn't exist in stdlib, FFI ask. v0 fallback: mtime polling per frame. |

## 5. Build phases

Phases re-sequenced against VISION's success criteria. Each
ships a verifiable deliverable.

**Spike status** (commits `80d977a` through `5671d7c`):
substrate + four pane stubs + EchoBackend + AgentHarness +
PermissionGate (consultative) + bus topics + input routing.
Validates the substrate. Production work continues from here
with the new architecture.

### Phase 1 — Proposal protocol + mechanical dispatch

**Goal.** PermissionGate moves from consultative to
structural — every mutation that touches a pane goes through
`ProposalDispatcher`. Stress test confirms deterministic
dispatch.

**Work.**
- `proposal_extractor.hl` — ProposalExtractor locus.
  Subscribes to AgentChunk + AgentTurnDone, runs streaming
  JSON parser over accumulated response text, emits
  `ide.proposal.in` events for each completed
  `iris.proposal{}` block.
- `proposal_dispatcher.hl` — ProposalDispatcher locus.
  Subscribes to `ide.proposal.in`. For each proposal: parse
  channel, derive (pane, op), check gate, dispatch via
  `dispatch_to_channel` table or emit denial. Audit-log
  every transition.
- Extend EchoBackend to emit a sample `iris.proposal{}`
  block in its response so the pipeline can be exercised
  end-to-end.
- Update `messages.hl` with `Proposal`, `Applied`, `Denial`
  types; `topics.hl` with `ProposalIn` / `ProposalApplied` /
  `ProposalDenied`.
- Update `main.hl`: Iris gains ProposalExtractor +
  ProposalDispatcher fields; the harness's bus path now
  routes through them.

**Acceptance.** A test scenario: AgentHarness submits a
prompt → EchoBackend emits text containing
`iris.proposal { channel: "ide.cmd.source.open", payload:
{...}, reason: "..." }` → ProposalExtractor publishes
`ide.proposal.in` → ProposalDispatcher validates + republishes
`ide.cmd.source.open` → SourcePane sees the open. Same
input run 100× produces identical dispatch sequences.

### Phase 2 — MCP server + read tools

**Goal.** External agents (Claude Code, scripting harnesses)
can connect to iris over MCP and call the read tools.

**Work.**
- `mcp_server.hl` — McpServer locus. JSON-RPC over stdio
  (initial transport). Holds a LocusRef back to Iris for
  reading pane state.
- Implement each read tool in AGENT_PROTOCOL.md §1:
  `iris.launch`, `pane.read`, `pane.focus_state`,
  `viz.snapshot`, `viz.find`, `bus.recent`,
  `permission.state`.
- `iris --mcp` flag: when set, iris starts in MCP-only mode
  (no GUI window). Window opens when `iris.launch` is called.
  When set without the flag, iris opens the window AND hosts
  the MCP server.
- An MCP "tools/list"-style discovery endpoint exposing the
  read-tool table to clients.

**Acceptance.** A script using `mcp` client lib (or
hand-rolled JSON-RPC) connects to iris, calls each read tool,
gets a sensible response. Tested against a real Claude Code
session that configures iris as an MCP server: it can launch
iris, read panes, take a `viz.snapshot`, see permission
state.

### Phase 3 — VizPane scene-host + lotus flower default scene

**Goal.** VizPane becomes a scene-hosting surface. iris ships
**`lotus.flower`** as the universal default scene — renders any
Hale locus tower as a beautiful animated flower. Works
identically for Hale source projects (the codebase IS the
model) and for non-Hale domains (the agent writes `.hl` that
models the domain; the flower renders the model). Domain
plugins specialize (replace / decorate / layer per §3.5) post
v0.1; the scaffold for them lands here.

**Status (2026-05-23).** Structural work landed in
`iris/lib/lotus_viz/` + `iris/lib/heron/` (vendored from
pond/heron). Verifies as 32/32 .hl files parse cleanly via
heron + the F.29 parent-edge resolution pipeline works
end-to-end at the .hl level. Visual rendering (real raylib
draws of petals on the dome) requires Phase 5 raylib being
exercised in the active loop + libtree-sitter installed on
the build host.

**Work — done:**
- ✓ `viz_scene.hl` — the `VizScene` interface (§3.5) plus
  `SceneChannelSpec` types. Lives at iris top-level.
- ✓ `lib/lotus_viz/scene.hl` — `LotusFlowerScene` implementing
  VizScene. Holds Parser (via TreeSitterHaleParser) +
  Animator + Camera. Render loop draws sphere-per-petal with
  scale / alpha / pulse-tint animations and ambient sway.
- ✓ `lib/lotus_viz/{model,aesthetics,layout,animator,parse,
  heron_adapter}.hl` — supporting library (model types,
  palette + timing constants, radial layout math, transient
  animation state, LocusTreeProvider interface, heron-backed
  TreeSitterHaleParser impl with F.29 parent-edge
  resolution).
- ✓ `lib/heron/` — vendored snapshot of pond/heron. Real
  tree-sitter grammar (not the previously-planned `std::ts`),
  19/19 stdlib coverage, Query support for highlight runs.
- ✓ Scene channels exposed by `lotus.flower`:
  `viz.scene.spotlight(id)`, `viz.scene.pulse(id, color)`,
  `viz.scene.draw_edge(a, b)`, `viz.scene.narrate(id, text)`.
  Spotlight + pulse fully wired through animator; draw_edge
  and narrate stubbed at the scene level (renderer-side
  work).
- ✓ `<workspace>/.iris/models/` convention pinned in code +
  DESIGN.md.
- ✓ `source_pane.hl` reads file content on SourceOpen and
  publishes SourceChanged with content, so the scene gets
  real input to parse via heron.

**Work — remaining for full Phase 3 acceptance:**
- VizPane upgrade to host an interface-typed `active_scene`
  field; route mode keys + camera keys + click coords through
  the scene. (LotusFlowerScene exists as a standalone locus;
  the VizPane → scene wiring still needs to land.)
- ProposalDispatcher channel-table extension for
  `ide.cmd.viz.scene.*` proposal routing into the active
  scene's `dispatch(channel, payload)`.
- MCP read tool extension: `viz.scene_info()` queries the
  active scene's `name() / domain() / channels_json() /
  state_json()` (the scene already exposes all four).
- Multi-file workspace parsing — currently the scene parses
  the one file SourcePane shows. fsnotify-driven workspace
  walk lands when SourcePane gains real viewport rendering
  (the same Phase 5 work that lights up SourcePane syntax
  highlighting via heron's Query surface).

**Acceptance.**
- ✓ Hale source → heron parses → TreeSitterHaleParser
  walks AST → Petals emitted with kind / projection / file /
  line → F.29 parent-edge resolution wires depth + parent_id
  → layout_tree assigns radial positions → scene.render walks
  + draws (per-petal sphere with animations).
  Verified via `examples/petal_extract.hl` at the data-flow
  level; visual verification waits on raylib + libtree-sitter
  on the build host.
- The agent (via MCP `viz.scene_info()`) can list the
  flower's accepted scene channels; an agent-emitted
  `viz.scene.spotlight` proposal updates the Animator's
  spotlit set (renderer applies the de-emphasis on the next
  frame). Verified at the .hl level.
- Open a non-Hale workspace; the agent writes
  `.iris/models/org.hl` modeling the workspace's domain via
  `source.write` proposal. SourcePane reads the file on
  SourceOpen; scene re-parses via heron; locus tower updates
  in the flower. End-to-end pipeline exists at the .hl
  level; visual verification follows the same dep gating as
  above.

### Phase 4 — Anthropic backend

**Goal.** A real LLM backend works end-to-end. Iris's
internal agent stops being a deterministic echo.

**Work.**
- Vendor `pond/http/client` → `iris/lib/http/`.
- Build `iris/lib/sse/` from stub to real: streaming-recv
  over `std::http`. Verify `std::http::recv_chunk` exists in
  the stdlib version we're targeting; if not, surface as a
  small hale ask.
- `agent_backend_anthropic.hl` — AnthropicBackend locus.
  Conversation log (`@form(vec)<Turn>`), Anthropic SSE
  stream, publishes `AgentChunk` + `AgentTurnDone` /
  `AgentError`.
- Workspace config picks active backend (`echo` vs
  `anthropic`).

**Acceptance.** Config `agent.backend = "anthropic"`. User
types a prompt; real LLM response streams in; if the response
contains an `iris.proposal{}` block, it round-trips through
ProposalExtractor → ProposalDispatcher → applied state
change.

### Phase 5 — Real raylib (compiler dependency)

**Goal.** `iris .` opens an actual window.

**Work.**
- **Blocked on `@ffi` Stage 1 in hale.** See
  COMPILER_FFI.md + BINDINGS_HANDOFF.md.
- When `@ffi` ships: author `pond/raylib` + `pond/pty` per
  BINDINGS_HANDOFF.md.
- iris swaps `import "lib/raylib"` → `import
  "vendor/pond/raylib"` (or keeps `lib/raylib` as a thin
  re-export shim). Same for `lib/pty`.
- Real frame loop. `Iris.run()` wraps the per-frame work
  (input poll → handle_key → frame_open → render panes →
  frame_close) in a cooperative loop until quit.

**Also lands in this phase** (because they need the real
window + input to be meaningful):
- **Status line** at the bottom of the window per USAGE.md.
  Five segments (workspace · backend/scene · focus/mode ·
  locks · activity-spinner). Reads from existing locus
  state every frame.
- **Mouse click forwarding** for VizPane: pane-relative
  screen coords → active scene's `on_click(x, y)`. Scene
  unprojects + hit-tests + reacts (per PLAN §3.5).
- **Toast overlay** in the top-right corner subscribing to
  `ide.proposal.denied` (and other denial events). Stack up
  to 3 visible toasts; collapse to "+N more" indicator
  above the third. Dismiss-all keybind (Ctrl+/).
- **Trinity layout + shell drawer** (VISION §13 — the answer
  to "is it still quad?"). The cockpit default is three
  always-visible panes (viz / source / agent) with shell as
  a toggleable drawer beneath the viz pane. Pieces:
  - **Default tiling.** Viz takes left ~2/3 full-height;
    source on right-column-top; agent on right-column-bottom.
    Layout locus owns the geometry; pane render is delegated.
  - **Shell drawer.** `` Ctrl+` `` toggles ShellPane's render
    inside the viz column. When open, slides up from the
    bottom of the viz column to its last-used height
    (default ~33% of column). Viz shrinks to fill the
    remainder, flower scene stays visible (just shorter).
    Drag handle between viz and shell allows live resize.
    `Ctrl+1` (shell focus) auto-opens the drawer if
    collapsed.
  - **Resizable dividers** between viz column and right
    column (vertical), between source and agent (horizontal),
    and between viz and shell when drawer is open
    (horizontal). Mouse drag + keyboard `Ctrl+Alt+H/J/K/L`
    nudge by 5%.
  - **Maximize keybind** (`Ctrl+M`): zooms the focused
    trinity pane to fill the window; the other two trinity
    panes collapse to a thin status strip along the bottom,
    still showing per-pane activity indicators (spinner,
    focus marker, last-proposal flash) so the user keeps
    ambient agent-activity signal. Shell drawer state
    preserved. `Ctrl+M` again restores.
  - **Layout presets** (`Ctrl+Shift+1..4`): source-heavy
    (source dominant, viz on side), viz-heavy (default
    proportions), agent-heavy (agent dominant for long
    chat reviews), shell-up (viz column split ~50/50 with
    shell). Per-workspace default in `.iris/session.json`.
  - **Agent-requested prominence.** New proposal channel
    `ide.cmd.layout.spotlight` lets the agent request brief
    visual emphasis ("popup viz — I just did something
    visual"); user retains override via PermissionGate.

  All four panes stay structurally live on the bus (the
  wedge preserved). Shell drawer state, layout proportions,
  and active preset all persist in
  `<workspace>/.iris/session.json`.

**Acceptance.** `iris .` opens a window with the trinity
layout: viz left ~2/3 full-height (showing the lotus flower),
source right-top, agent right-bottom. Shell collapsed by
default. Ctrl+Tab cycles focus through the three trinity
panes (skipping shell when collapsed). Ctrl+Q quits.
`` Ctrl+` `` toggles the shell drawer beneath the viz —
shell visible, PTY runs the user's shell, viz shrinks
vertically but flower stays visible. Drag handles resize.
Status line reflects workspace / backend / scene / focused
pane / lock state / shell-drawer state / agent activity in
real time. Clicking a viz node delegates to the active
scene. Denied proposals surface as toasts. `Ctrl+M` zooms
the focused trinity pane and collapses the others to a
bottom status strip; `Ctrl+M` again restores. Layout presets
switch via `Ctrl+Shift+1..4` and persist in
`.iris/session.json`. An agent-emitted
`ide.cmd.layout.spotlight` proposal raises the viz pane
briefly when the user hasn't locked layout changes.

### Phase 6 — First agent-modeled non-Hale domain demo

**Goal.** Empirically ground VISION §11.2 — show that the
substrate-invariance claim holds operationally: the agent
models a non-Hale domain in Hale, the lotus flower
renders the model, a domain expert recognizes the structure.
No domain-specific plugin scene required. The proof is the
agent's modeling skill + the universal flower, NOT
per-domain rendering code.

The phase that was previously the highest-risk (build a
brain3-specific topology renderer) becomes the lightest:
the heavy lifting is done by Phase 3's `lotus.flower` and
Phase 4's Anthropic backend. Phase 6 is the scripted demo
that combines them.

**Work.**
- Pick one of: multi-agent system, trading strategy, org
  chart, research paper. Probably **multi-agent system**
  per VISION §7 (strongest first-domain claim, audience is
  iris's MCP discoverers themselves).
- `examples/<domain>/` workspace fixture: the *artifact*
  the user came in with (a Python supervisor.py, an org
  chart CSV, a paper.md, whatever fits the chosen domain).
  NOT pre-written Hale — the agent writes the Hale in
  the demo.
- Agent system prompt addition (lands in `agent_harness.hl`
  config): "When the user asks about a non-Hale domain,
  your job is to write Hale that models the domain's
  structural shape. Each locus becomes a petal in the
  lotus flower. Use the discipline (locus vs type, F.22
  capacity, F.20 interfaces) to commit to structural
  choices. Write into `.iris/models/*.hl` via
  `source.write` proposals."
- Scripted demo (`iris.demos.run('model_<domain>')`):
  agent reads the artifact in the source pane, narrates
  its modeling choices, emits a sequence of `source.write`
  proposals creating the model files, watches the flower
  bloom petal by petal. Each step paced for reading.

**Acceptance.** A domain expert (someone who works in
multi-agent systems) opens iris on the fixture workspace,
asks the agent "model this," watches the agent write
Hale into `.iris/models/`, watches the flower render the
agent's model, and reports the structure matches their
mental model. Source pane shows the agent's `.hl` for
review. The proof is reproducible: the same demo, same
fixture, produces a recognizable model on first try.

**What this unlocks.** With Phase 6 done, every subsequent
domain is free — no per-domain build cost. The agent's
modeling skill is the substrate; the flower is the
universal rendering. Domain-specific plugin scenes (`replace`
shapes like `brain3.topology`) become post-v0.1 polish work
for domains whose visual conventions genuinely benefit from
specialization (per PLUGINS.md three-shapes catalog).

### Phase 7 — Persistence + polish

**Goal.** Session restore, workspace state, ergonomic
polish. Lower priority than the wedge work above.

**Work.**
- `session.hl` — SessionStore: subscribes to relevant
  events, debounces writes to `<workspace>/.iris/session.json`.
- Source pane state restore (which file was open + cursor
  position; the file's *content* is whatever's on disk now,
  per readout semantics).
- Layout state restore.
- Theme loading from config.
- Conversation: default is **start fresh**; `--resume`
  flag re-loads the last conversation from
  `agent_history.db`.
- `--no-restore` / `--no-agent` / `--mcp` / `--resume` flags.

**Acceptance.** Iris closed mid-session reopens to the same
panes / file paths / cursor positions. The file contents
reflect whatever's on disk *now* (not what was there when
iris closed). `--resume` brings back the last conversation.
Themes load from config.

## 6. v0.1 acceptance (against VISION §11)

iris ships v0.1 when:

1. **Two-channel bus + PermissionGate structurally
   enforced** (VISION §11.1). Adversarial proposal sequences
   that attempt to bypass the gate fail at the bus layer.
   Phase 1 deliverable.
2. **VizPane renders extracted locus structure for at least
   one non-Hale domain** (VISION §11.2). At least one of
   the Phase 6 demos works against a domain expert's mental
   model.
3. **External agents summon iris via MCP and operate
   concurrently** (VISION §11.3). Phase 2 (MCP server) +
   Phase 5 (real raylib) deliverable. A Claude Code session
   launches iris, reads panes, emits proposals, sees the
   user's parallel edits.
4. **Mutations come through structured output, not MCP
   tool-calls** (VISION §11.4). Phase 1 deliverable. Stress
   harness confirms deterministic dispatch.

Phases 1–6 land v0.1. Phase 7 is v0.2+.

## 7. Out of scope (for v0)

- Plugins / extension API beyond what AgentBackend already
  provides. PLUGINS.md remains as the deferred design.
- **4-pane → tabbed-window restructure** (one pane visible
  at a time, browser-style tabs to switch). Considered
  during the 2026-05-23 design session and ruled per
  VISION §13: tabs preserve the two-channel bus
  mechanically but kill the concurrent-work *signal* (the
  user loses the live "agent is updating viz while I edit
  source" wedge). Visual density handled via dynamic
  prominence in Phase 5 instead.
- Multi-window iris (one window per workspace process).
- Multi-user iris (two humans sharing one session — possible
  with the bus, not implemented).
- Cross-iris federation (multiple iris instances coordinating
  over WebSocket via `TransportBridge`).
- Mouse-driven UI beyond viz pane orbit + scrollback
  selection.
- Tab strips inside panes (one open thing per pane initially).
  Distinct from the 4-pane → tabbed restructure above —
  tabs *within* a single pane don't break the wedge;
  scoped for post-v0.1.
- Domain-specific plugin `VizScene`s (brain3.topology,
  <demo-app>.strategy, etc.). The `lotus.flower` default + the
  agent's modeling skill cover every domain at decent
  fidelity for v0.1; plugin scenes are post-v0.1 polish
  for domains whose visual conventions justify it.
- LSP server (we may add LSP client in Phase 7).
- Verification / proof tooling. Closure tests +
  PermissionGate + structural typing live at the Hale
  layer; iris surfaces them but adds no new verification
  surface.
- Non-Linux platforms initially. macOS / Windows follow when
  raylib's portability earns its keep.

## 8. Style discipline

- Match `hale/spec/styleguide.md` exactly. Every locus
  matches one of the six-pattern catalog. "Module of free
  fns" → namespace lotus.
- Track friction in `FRICTION.md` whenever a language gap
  forces a workaround. Don't code around silently.
- Bus subject names are a closed surface — adding one updates
  §3.3 first.
- MCP read tool surface is a closed contract —
  AGENT_PROTOCOL.md changes first.
- Proposal channel-payload table is a closed contract —
  AGENT_PROTOCOL.md changes first.
- Use `BytesBuilder` for any growing buffer.
- F.27 `violate` for substrate-level failures (PTY death,
  render surface lost, FFI crash). `fallible(E)` for value
  errors at the proposal-parse boundary.
- **Local libs first** under `iris/lib/`. Don't add to
  parallel `pond/` repo from iris sessions.
- **Don't conflate read and write paths in the agent
  interaction surface.** If a workload seems to need an
  agent-callable write tool, the answer is almost always
  "use a proposal instead." Reach for FRICTION.md before
  reaching for a tool.

## 9. Open questions

To resolve in context. Tracked here so they don't slip.

1. **MCP transport.** stdio is canonical for Claude Code;
   TCP socket might be needed for cross-machine summons.
   Phase 2 starts with stdio.

2. **AST extraction for `.hl` source.** Resolved — shipped
   as `pond/heron` (tree-sitter grammar + Hale `@ffi`
   wrapper), vendored at `iris/lib/heron/`. Replaces the
   earlier plan to depend on a stdlib `std::ts` wrapper.
   See §4 Substrate prerequisites + the lotus_viz adapter
   in `lib/lotus_viz/heron_adapter.hl`.

3. **Proposal preconditions.** AGENT_PROTOCOL.md sketches an
   optional `precondition` field. Concrete shapes
   (no-concurrent-edit, viz-mode-is-X, pane-unlocked) emerge
   as workloads need them.

4. **Streaming vs turn-end proposal dispatch.** Default
   greedy-streaming so proposals dispatch as they parse.
   Per-workspace override available for "apply all-or-nothing
   per turn." Decide default in Phase 1.

5. **brain3 backend.** Lands after MCP read tools (Phase 2)
   because brain3's task events feed naturally into the
   proposal pipeline (Phase 1). Brain3-specific viz (node
   DAG) goes through the standard VizPane extraction path.

6. **System prompt for the internal agent.** What does the
   agent know about iris at boot? Tool list is mechanical
   (read AGENT_PROTOCOL.md §1); tone, conventions, proposal
   schema awareness, persona are open. Likely starts terse
   and grows from friction.

7. **Conversation persistence.** Per-workspace SQLite
   (`agent_history.db`) vs in-memory. Default per-workspace
   for continuity with `--fresh-agent` to override. Decide
   in Phase 4 when AnthropicBackend lands.

8. **VizPane interaction.** When the user clicks / focuses
   on a viz node, what happens? Likely: source pane navigates
   to the corresponding source location (for AST mode) or
   the agent narrates the node (for extracted mode). Decide
   in Phase 3.

9. **Multi-instance.** `iris .` twice on the same workspace —
   refuse or open read-only? Likely refuse. Decide in Phase 7.

## 10. Pickup pointers

Coming into this cold:

- **Start here:** [`VISION.md`](./VISION.md), then this file
  end-to-end, then `AGENT_PROTOCOL.md`.
- **Where we are:** check the latest completed phase's
  "Acceptance" against the codebase. Pick up from the next
  incomplete phase. Spike work in commits `80d977a` through
  `5671d7c` (Sept 2026); production phasing starts from §5
  Phase 1.
- **Architectural commitments:** §2 First principles is
  non-negotiable. Anything that violates them is a redesign
  moment, not an implementation choice.
- **Hale reference:** `../hale/spec/` — particularly
  `styleguide.md`, `design-rationale.md` (F.20 interfaces,
  F.27 violate, F.29 locus-typed param fields), and
  `forms.md`.
- **Plugin / interface contracts:** [`PLUGINS.md`](./PLUGINS.md)
  (deferred build-time manifest); the AgentBackend interface
  itself lives in `agent_backend.hl` and is documented in
  §3.5 above.
- **FFI contracts:** [`COMPILER_FFI.md`](./COMPILER_FFI.md) +
  [`BINDINGS_HANDOFF.md`](./BINDINGS_HANDOFF.md).
- **Agent contracts:** [`AGENT_PROTOCOL.md`](./AGENT_PROTOCOL.md).
- **Friction log:** [`FRICTION.md`](./FRICTION.md) — what
  hasn't worked. Read before architecting anything.

## 11. Status

Spike-complete; production phasing begins. Next move:
Phase 1 — proposal protocol + mechanical dispatch. The spike
code (commits `80d977a..5671d7c`) provides the substrate this
phase builds on; the direct `AgentHarness → backend →
AgentPane` round-trip becomes
`AgentHarness → backend → AgentPane (text) + ProposalExtractor
(proposals) → ProposalDispatcher → ide.cmd.<pane>.*`.
