# iris

A **modeling cockpit for structured information** — built in
Hale. A trinity of always-visible panes (viz, source, agent)
hosts concurrent human+agent work on the locus structure of
whatever's being modeled: code, agent topologies, financial
strategies, research papers, organizational hierarchies. A
fourth pane (shell) lives as a toggleable drawer beneath the
viz pane, summoned with `` Ctrl+` `` when you need it.

Not an IDE. Not a chat UI. Not a notebook. Not a whiteboard.
A cockpit through which a human and an agent concurrently work
with the structure of structured information at any substrate.

The load-bearing commitment: **two independent input channels on
the bus**. User keystrokes flow on `ide.input.<pane>.*`
(focus-routed); programmatic commands flow on `ide.cmd.<pane>.*`
(always-on). The agent drives panes the user isn't focused on
without stealing focus, because the channels never collide.

The wedge: **concurrent human+agent work on structured
information, with structural permission gating and live
locus-structure visualization.** Nothing else ships this
combination.

The modeling notation is Hale itself. For Hale source
projects, the codebase IS the model — iris is an IDE out of
the box. For non-Hale domains (agent topologies, trading
strategies, org charts, research papers), the agent writes
Hale on the fly that models the domain; iris renders the
agent's model as a lotus flower (the default `VizScene`).
One notation, one default visualization, every domain. See
[`VISION.md §4`](./VISION.md) for the full framing.

## Documents

| File | What it covers |
|---|---|
| [`VISION.md`](./VISION.md) | Strategic frame: what iris is, who it's for, what makes it defensible |
| [`USAGE.md`](./USAGE.md) | User-facing surface: what you see, what you do, vocabulary, typical session |
| [`PLAN.md`](./PLAN.md) | Implementation roadmap: phases, architecture, deliverables |
| [`AGENT_PROTOCOL.md`](./AGENT_PROTOCOL.md) | Contract for external/internal agent integration: MCP read tools + structured-output proposal schema |
| [`COMPILER_FFI.md`](./COMPILER_FFI.md) | Binding surface contract for raylib + PTY (lands as `pond/raylib` / `pond/pty` via `@ffi`) |
| [`BINDINGS_HANDOFF.md`](./BINDINGS_HANDOFF.md) | How-to-pick-up brief for whoever lands the pond bindings once `@ffi` ships |
| [`PLUGINS.md`](./PLUGINS.md) | Future plugin surface (deferred per VISION §13) |
| [`FRICTION.md`](./FRICTION.md) | Friction log + useful findings |

Read VISION first to understand *what* iris is. Read USAGE
next to understand *how to use* it. Everything else is for
people building iris.

## Status

Phase 1–5 structural work landed; build currently blocked on
F.10 (compiler-side codegen issue with cross-seed type names
in fn signatures — see FRICTION.md). All 23 top-level .hl
files + lib/lotus_viz (heron-backed flower) + lib/heron
(vendored tree-sitter grammar) typecheck cleanly via
`hale check`. heron itself is verified end-to-end against
libtree-sitter — the .hl pipeline is structurally complete;
just waiting on F.10 to actually build the binary.

Phases shipped (per PLAN.md §5):

- **Phase 1** — proposal protocol + mechanical dispatch
- **Phase 2** — MCP server + read tools
- **Phase 3** — VizPane scene-host + LotusFlowerScene
  (heron-backed; replaces the old HaleLocusScene stub)
- **Phase 4.a** — AnthropicBackend (HTTPS request-response)
- **Phase 5.a–c** — real raylib via @ffi; interactive frame
  loop; headless --mcp; status line; mouse forwarding

Remaining for v0.1:

- F.10 closure (compiler-side) → iris builds
- #30 SourcePane viewport rendering (highlight pipeline data-
  flow complete; visual render is Phase 5 viewport work)
- #31 placement block to pin Renderer once M:N pool active

## Dependencies

iris is self-contained at v0. All FFI bindings, new substrate
helpers, and snapshots of pond libs live under `iris/lib/`.
No modifications to the parallel `pond/` repo from iris
sessions; no `hale fetch` to `vendor/`. Snapshots of pond
libs we use (e.g. `pond/heron`, eventually `pond/http/client`)
are one-time copies into `iris/lib/`, treated as forks from
that moment. See PLAN.md §4 for the strategy.

Build host needs **libtree-sitter** installed
(`apt install libtree-sitter-dev` / `brew install tree-sitter`)
for the heron-backed parsing pipeline. raylib is already
covered by `lib/raylib`'s @ffi bindings.

## Distribution model

iris is **summoned, not switched-to** (VISION §5). Developers
don't move from VSCode to iris. Their existing agent (Claude
Code, Cursor, Windsurf, …) discovers iris on PATH, reads
`iris --help`, calls `iris.launch(workspace=...)` via MCP
when a task fits. The agent is the marketing channel.

See [`MCP_SETUP.md`](./MCP_SETUP.md) for the concrete steps to
wire iris into your agent shell — plus the catalog of demos
your agent can drive via `iris.demos.list` /
`iris.demos.run`. "Show me iris" works once the MCP config is
in place.

Planned install paths (post-v0.1):

```bash
brew tap hale-lang/iris && brew install iris   # macOS
curl -sSf https://get-iris.sh | sh                # Linux
cargo install --git ... iris                      # source
```

## Running (planned)

```bash
iris .                       # open cwd as workspace
iris ~/code/myproject        # specific dir
iris --mcp                   # MCP-server-only mode (no GUI)
iris . --no-restore          # fresh session
iris . --no-agent            # start with agent disabled
```

## Workspace state

Per-workspace state in `<workspace>/.iris/`:

```
.iris/
├── config.json          # workspace-local config overrides (committed)
├── session.json         # last layout, open files, cursor positions
├── permissions.json     # PermissionGate state
├── proposals.log        # audit log of every dispatched / denied proposal
└── agent_history.db     # conversation persistence (backend-managed)
```

User-global config: `~/.config/iris/config.json`. Workspace
overrides user, field-by-field.
