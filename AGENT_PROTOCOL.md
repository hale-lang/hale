# Agent protocol — read tools + write proposals

> **Status:** source-of-truth contract for how external agents
> (Claude Code, Cursor, Windsurf, etc.) interact with a running
> iris instance, and how iris's internal agent emits mutations.
> Same role `COMPILER_FFI.md` plays for FFI: edits here are
> binding; implementations track this doc.

## Why this exists

VISION §6 fixes the split:

> Reads are in-band MCP tools. Writes are out-of-band
> structured-output proposals processed by a mechanical outer
> loop.

The motivation, distilled from brain3's production experience:
**agents calling tools to mutate state is nondeterministic.**
The agent might call the tool, might not, might call it twice,
might call it with subtly different args across runs of the
same prompt. For *reads* that's fine — the agent decides what
to look at; looking-or-not-looking doesn't change state. For
*writes* it's structurally broken: state mutations gated by
model-decision-to-call inherit the model's nondeterminism.

iris's answer: split the channels.

- **Reads** = MCP tools the agent calls. Each call is
  side-effect-free; nondeterminism is contained to "did the
  agent decide to look."
- **Writes** = structured-output proposals the agent emits in
  its response text. A mechanical outer loop parses, validates,
  dispatches. Nondeterminism is contained to "did the agent
  decide to propose"; the dispatch from a proposal is
  deterministic.

This applies to BOTH external agents (via MCP) and iris's
internal agent (via the harness). The contract is one-sided
about reads vs. writes; both flavors of agent see the same
boundary.

---

## Surface 1 — MCP read tools

iris hosts an MCP (Model Context Protocol, JSON-RPC over stdio)
server. External agents configure iris as an MCP server in
their tooling and gain these read-tools:

### `iris.launch(workspace: String) -> LaunchResult`

Open iris's GUI window against the given workspace path. If
iris is already launched against this workspace, returns the
existing handle (no second window). If not, opens the window
and returns a handle the agent can use for subsequent tool
calls.

```json
{
  "workspace": "/home/riley/code/myproject",
  "result_handle": "iris-session-7f3a"
}
```

The launch entry point is what makes iris "summoned, not
switched-to" (VISION §5). External agents discover iris via
their MCP registry; they decide whether the task fits; they
launch iris when it does.

### `pane.read(pane_id: String, range?: Range) -> PaneSnapshot`

Read the current content of a pane.

| pane_id | What's returned |
|---|---|
| `"source"` | File buffer at current cursor, viewport text, file path, cursor pos |
| `"shell"` | Last N lines of scrollback (default 200, configurable via `range`) |
| `"viz"` | Locus structure snapshot (delegates to `viz.snapshot`) |
| `"agent"` | Conversation history (turns + chunks + tool_calls + proposals) |

Optional `range` constrains the response:

```json
{
  "pane_id": "source",
  "range": { "kind": "lines", "start": 40, "end": 60 }
}
```

### `pane.focus_state() -> FocusState`

```json
{
  "focused_pane": "agent",
  "visible_panes": ["shell", "source", "viz", "agent"],
  "locked_panes": { "shell": { "read": true, "write": false } }
}
```

### `viz.snapshot() -> VizStructure`

Return the locus structure currently rendered in VizPane. For
`.hl` source files: the parsed locus tower. For other artifacts:
whatever extraction the agent has fed into `ide.cmd.viz.*`.

```json
{
  "domain": "hale",
  "root": {
    "name": "Iris",
    "kind": "locus",
    "children": [
      { "name": "ShellPane", "kind": "locus", "children": [...] },
      { "name": "SourcePane", "kind": "locus", "children": [...] }
    ],
    "contracts": [...],
    "bus_topics": [...]
  }
}
```

### `viz.scene_info() -> SceneInfo`

Discover what scene is currently active in the VizPane and
what proposal channels it accepts. Essential for any agent
that wants to drive the viz beyond camera + mode primitives.

```json
{
  "name": "brain3.topology",
  "domain": "brain3",
  "channels": [
    {
      "channel": "ide.cmd.viz.scene.add_node",
      "payload_schema": "{ type: 'object', properties: { id: 'string', kind: 'string', parent: 'string?' }, required: ['id', 'kind'] }",
      "description": "Add a node to the topology under the named parent."
    },
    {
      "channel": "ide.cmd.viz.scene.bind_idea",
      "payload_schema": "...",
      "description": "Bind an idea to a node."
    }
  ]
}
```

The scene's channel list is the agent's API for that domain
of viz. Different active scenes expose different channels.

### `viz.find(query: String) -> [VizMatch]`

Locate a locus / topic / contract / binding by name or pattern
in the viz structure.

```json
{
  "query": "AgentHarness",
  "matches": [
    {
      "path": "Iris/AgentHarness",
      "kind": "locus",
      "source_file": "agent_harness.hl",
      "source_line": 17
    }
  ]
}
```

### `bus.recent(subject: String, n: Int) -> [BusEvent]`

Read the last N events on a bus subject. Useful for the agent
to verify a proposal was dispatched or to understand recent
state changes.

```json
{
  "subject": "ide.cmd.source.open",
  "n": 5,
  "events": [
    {
      "ts": "2026-05-22T14:33:01Z",
      "payload": { "path": "adapter.hl", "line": 42 }
    }
  ]
}
```

### `permission.state() -> PermissionMatrix`

```json
{
  "panes": {
    "shell":  { "read": true,  "write": false },
    "source": { "read": true,  "write": true  },
    "viz":    { "read": true,  "write": true  },
    "agent":  { "read": true,  "write": true  }
  },
  "recent_denials": [
    {
      "ts": "2026-05-22T14:30:12Z",
      "channel": "ide.cmd.shell.exec",
      "why": "shell pane locked"
    }
  ]
}
```

### Conventions across all read tools

- **Idempotent.** Calling twice returns the same data (modulo
  intervening state changes). No side effects.
- **Cheap.** Tools must complete in <100ms in steady state.
  Agents may call them frequently to maintain context.
- **JSON-shaped.** Every payload is a JSON object with
  string-keyed fields. No binary, no streaming.
- **Snapshot semantics.** The data returned is a coherent
  snapshot at call time — fields within one response are
  internally consistent.

---

## Surface 2 — Structured-output proposals

The agent emits mutations as **structured-output proposals** in
its response stream. iris's mechanical outer loop reads them,
validates against `PermissionGate`, dispatches.

### Proposal schema

```
iris.proposal {
  channel: "ide.cmd.source.write",
  payload: { file: "adapter.hl", insert_at_line: 42, content: "..." },
  reason: "adding null check on conn_fd"
}
```

Embedded in the agent's response text as a tagged block. The
recognizer regex (canonical form):

```
^iris\.proposal\s*\{\s*$
... JSON-shaped body ...
^\}\s*$
```

Multiple proposals per response are allowed; each is parsed
and dispatched independently.

### Fields

| Field | Type | Required | Notes |
|---|---|---|---|
| `channel` | String | yes | The `ide.cmd.<pane>.<action>` subject to dispatch to. |
| `payload` | Object | yes | The typed payload for that channel. Schema depends on channel — see "Channel-payload table" below. |
| `reason` | String | yes | Human / model-readable rationale. Logged to the bus event + shown in agent pane echo. Empty string allowed but discouraged. |
| `id` | String | no | Optional client-generated id for tracking. iris generates one if absent. |
| `precondition` | Object | no | Optional check the dispatcher runs before applying — e.g. `{ "subject": "ide.event.source.changed", "since": "...", "must_be_empty": true }` to assert no concurrent edit. |

### Channel-payload table

The dispatchable channels split into two families: **iris
core primitives** (always available; payloads fixed by this
doc) and **scene-defined channels** (per-scene; payloads
discovered at runtime via `viz.scene_info()`).

**Core primitives:**

| `channel` | `payload` shape |
|---|---|
| `ide.cmd.source.open` | `{ "path": String, "line": Int? }` |
| `ide.cmd.source.goto` | `{ "line": Int, "col": Int? }` |
| `ide.cmd.source.write` | `{ "file": String, "insert_at_line": Int, "content": String }` — write hits **disk**; source pane reflects via fsnotify. Also `replace_range`, `delete_range`, etc. (TBD as workloads surface). |
| `ide.cmd.shell.exec` | `{ "cmd": String, "args": [String] }` |
| `ide.cmd.viz.camera` | `{ "position": Vec3, "target": Vec3 }` |
| `ide.cmd.viz.mode` | `{ "mode": "bulk" \| "harmonic" \| "resolution" }` |
| `ide.cmd.viz.focus` | `{ "id": String }` |
| `ide.cmd.viz.set_scene` | `{ "scene": String }` — switch the active viz scene by name. |
| `ide.cmd.gate.set` | `{ "pane": String, "read": Bool?, "write": Bool? }` — only callable by the user; denied for agent. |

**Scene-defined channels** match the pattern
`ide.cmd.viz.scene.<method>` and are dispatched into the
active VizScene's `dispatch(channel, payload_json)`. Each
scene declares its channels + payload schemas via
`channels()` (see PLAN §3.5); the agent discovers them via
the MCP `viz.scene_info()` tool. Example: the
`brain3.topology` scene exposes
`ide.cmd.viz.scene.add_node`, `…bind_idea`,
`…connect_nodes`, etc. — <demo-app>'s `strategy` scene exposes
a completely different set.

This is by design: viz is the most domain-specific surface
in iris; baking domain-specific channels into the core
contract would tightly couple iris to whatever first domains
we shipped. The split keeps the core primitive table closed
and lets domain plugins own their viz API.

### Determinism guarantees

Once a proposal is *emitted* (the LLM's choice), the dispatch
from proposal → applied state change is **deterministic**:

- Same proposal → same `PermissionGate.allows()` decision (the
  gate is local state, no randomness).
- Same proposal → same channel + payload → same bus dispatch.
- The pane subscriber's handler is deterministic Hale code.

This is the property VISION §11 success criterion 4 names:
"a stress harness running the agent 100 times on the same
input produces deterministic mutation sequences" *given the
agent emitted the same proposals*. Agent decision-making
remains LLM-nondeterministic; everything downstream is not.

---

## Internal architecture

Three new loci land in iris to implement this contract.
PLAN.md §3 has the full locus tower; this section covers the
new pieces.

### `McpServer`

JSON-RPC server over stdio (or alternative transport — see Open
questions). Holds references to the iris locus tree and answers
read-tool calls by reading current state.

```hale
locus McpServer {
    params {
        iris: Iris;            // F.29 LocusRef back to the root
        transport: McpTransport = McpTransport { mode: "stdio" };
    }
    run() {
        loop {
            let req = self.transport.next_request() or break;
            let resp = self.dispatch(req);
            self.transport.send_response(resp);
            yield;
        }
    }
    fn dispatch(req: McpRequest) -> McpResponse { ... }
}
```

Each read tool corresponds to a `fn handle_<tool>(args)` method
on `McpServer`; `dispatch` routes by method name.

### `ProposalExtractor`

Subscribes to the agent's chunk stream, accumulates response
text, runs a streaming parser that detects `iris.proposal { ... }`
blocks, and publishes each completed proposal onto
`ide.proposal.in`.

```hale
locus ProposalExtractor {
    params {
        accum: std::bytes::BytesBuilder
             = std::bytes::BytesBuilder { initial_cap: 16384 };
    }
    bus {
        subscribe AgentChunk     as on_chunk;
        subscribe AgentTurnDone  as on_turn_done;
        publish   ProposalIn;     // ide.proposal.in
    }
    fn on_chunk(c: Chunk) {
        self.accum.append(std::bytes::from_string(c.text));
        // Greedily scan for complete proposal blocks; emit each.
    }
    fn on_turn_done(t: TurnComplete) {
        // Final pass: drain any complete blocks at turn-end.
        // Any partial-block tail is logged as a warning.
        let _ = t;
    }
}
```

### `ProposalDispatcher`

Subscribes to `ide.proposal.in`, validates each proposal
against `PermissionGate`, and either republishes to the named
channel or emits a denial event.

```hale
locus ProposalDispatcher {
    params {
        gate: PermissionGate;
        audit: AuditLog;
    }
    bus {
        subscribe ProposalIn as on_proposal;
        publish   ProposalApplied;   // ide.proposal.applied
        publish   ProposalDenied;    // ide.proposal.denied
        // plus dynamic dispatch to ide.cmd.<pane>.<action>
    }
    fn on_proposal(p: Proposal) {
        let pane = pane_for_channel(p.channel);
        let op   = op_for_channel(p.channel);
        if !self.gate.allows(pane, op) {
            self.audit.record_denial(p);
            ProposalDenied <- Denial { proposal: p, why: "..." };
            return;
        }
        // Republish onto the named channel — pane subscribers
        // receive and apply.
        dispatch_to_channel(p.channel, p.payload);
        ProposalApplied <- Applied { proposal: p };
    }
}
```

`dispatch_to_channel` is the mechanical fanout: known channels
map to known publish sites. New channels added to the table as
PLAN.md §3.3 grows.

**Note on `source.write`.** Source mutations target the
**file on disk**, not an in-memory buffer (the source pane is
readout-only per PLAN §3.4). ProposalDispatcher's
`source.write` handler does the disk write directly via
`std::io::fs::*`; the source pane's fsnotify watcher detects
the change and re-reads if the modified file matches what
it's currently displaying. This makes proposals and the
user's external-editor edits compose naturally: both hit the
same disk state; iris reflects both.

**Note on scene channels.** Scene-defined channels
(`ide.cmd.viz.scene.<method>`) route to the active scene's
`dispatch(channel, payload)` rather than to a fixed
publish site. The dispatcher's table has a single entry for
the `ide.cmd.viz.scene.*` prefix; the active scene resolves
the rest.

### Why three loci, not one

Separation of concerns:

- `McpServer` deals with one specific transport (JSON-RPC over
  stdio) and one specific direction (external agent → iris).
- `ProposalExtractor` deals with one specific format (tagged
  blocks in response text) and one specific source (the agent
  chunk stream — currently from one backend, later possibly
  multiplexed).
- `ProposalDispatcher` deals with one specific concern
  (gate-then-dispatch) and is the single point of enforcement.

Each is independently testable. PermissionGate enforcement
lives in exactly one place; any path that wants to mutate iris
goes through ProposalDispatcher.

---

## Bus subject schema (proposal channel additions)

New subjects on top of PLAN.md §3.3:

| Subject | Publisher | Subscriber | Payload |
|---|---|---|---|
| `ide.proposal.in` | ProposalExtractor (or any agent) | ProposalDispatcher | `Proposal` |
| `ide.proposal.applied` | ProposalDispatcher | AgentPane (display), audit log | `Applied` |
| `ide.proposal.denied` | ProposalDispatcher | toast UI, audit log | `Denial` |

Topic decls (see `topics.hl`):

```hale
topic ProposalIn       { payload: Proposal; subject: "ide.proposal.in"; }
topic ProposalApplied  { payload: Applied;  subject: "ide.proposal.applied"; }
topic ProposalDenied   { payload: Denial;   subject: "ide.proposal.denied"; }
```

Payload type:

```hale
type Proposal {
    id:        String = "";
    channel:   String;          // required
    payload:   String;          // required — JSON-encoded payload
                                // (typed dispatch happens in
                                // dispatch_to_channel)
    reason:    String = "";
    ts:        Int    = 0;      // monotonic timestamp
}
```

The `payload` is held as a JSON string at this layer because
Hale's type system doesn't have a heterogeneous-record type
to model "any one of N different payload shapes." Typed
deserialization happens in `dispatch_to_channel` against the
known table.

---

## Summoning — how external agents launch iris

The flow VISION §5 describes:

```
external agent (Claude Code, Cursor, ...)
    │
    │ via its MCP registry: discovers `iris` is configured
    │
    │ calls iris.launch(workspace="/home/...")
    ▼
iris (or iris startup)
    │
    │ if not running: spawn iris GUI; expose MCP server
    │ if running:     attach to existing session
    │
    └─→ returns LaunchResult { handle: "..." }

(thereafter)

external agent
    │
    │ pane.read / viz.snapshot / bus.recent / ...
    │   (read tools — direct MCP calls)
    │
    │ emits `iris.proposal { ... }` blocks in response text
    │   (writes — flow through ProposalExtractor →
    │   ProposalDispatcher → ide.cmd.<pane>.*)
    ▼
iris dispatches; pane state changes; user sees the result
```

Two iris startup paths:

1. **CLI-launched.** User runs `iris .` from a shell. iris
   opens its window AND starts its MCP server. External agents
   can connect to the MCP server independently.
2. **MCP-summoned.** External agent calls `iris.launch(...)`.
   iris starts (window + MCP server). External agent gets the
   handle and continues.

Both end in the same state. The launch tool is idempotent —
calling it against an already-running iris returns the existing
handle.

---

## Open questions

To resolve as the work lands.

1. **MCP transport.** stdio is the canonical Claude-Code-shape;
   alternative is a TCP socket (for cross-machine summons).
   v0 starts with stdio. TCP follows when a workload needs it.

2. **Proposal format alternatives.** Tagged blocks
   (`iris.proposal { ... }` in response text) are the v0 shape.
   Alternatives considered: (a) dedicated tool_use channel
   (would re-introduce nondeterminism via tool-call shape);
   (b) anthropic's structured-output mode if/when it stabilizes
   (couples to one backend). Tagged blocks are
   backend-agnostic and stay textual; resist the temptation to
   "improve" them by binding to a particular API's structured
   output until a workload demands.

3. **Proposal preconditions.** The optional `precondition`
   field is sketched but not specified. Likely shapes:
   "no concurrent edit since timestamp," "current viz mode is
   X," "pane is unlocked." Add as workloads surface them; keep
   the schema open-ended for now.

4. **Streaming parse vs. turn-end parse.** ProposalExtractor's
   default is greedy-streaming so proposals dispatch as they
   complete mid-turn. Some scenarios want turn-end batching
   (apply all-or-nothing per turn). Configurable per
   workspace, defaulting to streaming.

5. **Determinism stress tests.** Success criterion 4 from
   VISION §11 wants a harness. Design: same prompt × 100,
   recorded responses, replay through ProposalExtractor +
   ProposalDispatcher, assert identical dispatch sequences.
   Lands alongside the dispatcher implementation.

6. **Idempotency of dispatched proposals.** Two identical
   proposals applied back-to-back — what happens? Idempotent
   for some channels (`gate.set`), state-mutating for others
   (`source.write`). Each pane subscriber decides per its own
   semantics; proposal layer doesn't enforce.

7. **Agent-side schema discovery.** How does an external agent
   know what channels/payloads iris accepts? Options: (a)
   `iris --help` includes the channel-payload table, agent
   reads at startup; (b) MCP `tools/list`-style discovery
   call; (c) hard-coded knowledge baked into agent's system
   prompt. (a) is the lowest-friction starting point.

8. **Brain3 / external orchestration interaction.** If iris is
   running as the cockpit while brain3 is the agent backend
   (per PLUGINS.md's future `brain3` AgentBackend), how do
   brain3's task events map to proposals? Probably: brain3
   tasks reach iris through the AgentHarness like any other
   backend; their tool actions become proposals at extraction
   time. Defer concrete spec until brain3 backend lands.
