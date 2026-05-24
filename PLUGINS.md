# Plugin surface — deferred

> **Status:** plugin surface (build-time manifest + generated
> registry + config-driven plugin selection) is deferred per
> VISION §13 ("plugin surface gated on a workload that needs
> it"). This document remains as the design sketch; nothing in
> it is implemented today.
>
> The **`AgentBackend` interface** is load-bearing and lives in
> [`agent_backend.hl`](./agent_backend.hl) (documented in
> PLAN.md §3.5). The **`Pane` interface** is similarly
> load-bearing and lives in [`pane.hl`](./pane.hl) (PLAN.md
> §3.2). These interfaces exist now; what's deferred is the
> *plugin-discovery and runtime-selection machinery* around
> them. v0 hardcodes the active backend in `AgentHarness`.

## Why this is deferred

The plugin model adds:

- A `plugins.toml` manifest at the iris repo root.
- A build-script pre-pass that reads the manifest and
  generates `iris/build/registry.hl`.
- A runtime selector that picks the active backend / tool /
  pane subset from per-workspace config.

The cost: new build tooling, generated-file gitignore, a
config schema, doc surface. The value: third parties can ship
plugins without editing iris core.

VISION's stance: the value lands when there's a real
third-party plugin author asking for it. Until then, the
hardcoded shape is simpler and lets iris move faster on the
load-bearing wedge work.

## Pluggable interfaces (load-bearing today)

Three interfaces are interface-shaped and continue to be.
Anyone who eventually writes a plugin will satisfy these
F.20-structurally. The *manifest* / runtime selection
machinery is deferred; the interfaces are not.

### `AgentBackend`

Lives in [`agent_backend.hl`](./agent_backend.hl). The active
backend in `AgentHarness` is hardcoded at v0. Current
implementations: `EchoBackend`. Planned: `AnthropicBackend`
(Phase 4), `Brain3Backend` (after Phase 4).

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

**Note vs. earlier sketches**: no `submit_tool_result`. Writes
are out-of-band proposals (see AGENT_PROTOCOL.md), not tool
callbacks.

### `VizScene`

Lives in `viz_scene.hl` (lands in Phase 3 — PLAN §5). The
viz pane hosts exactly one active scene at a time; switching
scenes is config or proposal-driven. Each scene defines what
to render, what data feeds it, and what agent API it exposes.
iris core ships **`lotus.flower`** as the universal default
— renders any Hale locus tower as an animated lotus.
Domain plugins specialize on top of (or in place of) the
flower.

```hale
interface VizScene {
    fn name() -> String;        // "lotus.flower", "brain3.topology", ...
    fn domain() -> String;      // "hale", "brain3", "<demo-app>", ...
    fn install();
    fn uninstall();
    fn render(s: RenderSurface);
    fn camera() -> Camera3D;
    fn set_camera(c: Camera3D);
    fn channels() -> [SceneChannelSpec];
    fn dispatch(channel: String, payload_json: String);
    fn on_source_changed(file: String, content: String);
    fn on_click(screen_x: Int, screen_y: Int);
}

type SceneChannelSpec {
    channel:        String;
    payload_schema: String;
    description:    String;
}
```

The agent discovers the active scene's API via the MCP read
tool `viz.scene_info()` — returns name, domain, and the
channel table the scene accepts. Proposals on
`ide.cmd.viz.scene.<method>` channels route through
ProposalDispatcher into the scene's `dispatch`.

#### The flower as universal floor

`lotus.flower` works for any Hale locus tower. Because
Hale is iris's universal modeling notation (per VISION
§1 + §4), the flower works for any domain:

- **Hale projects.** Flower renders the codebase's own
  locus tower.
- **Non-Hale domains.** Agent writes Hale that models
  the domain into `<workspace>/.iris/models/*.hl`; flower
  renders the agent's model.

This makes the flower the *floor*. Every domain gets
decent visualization on day one with no plugin authoring,
because The Design's substrate-invariance claim says every
domain has a locus tower the agent can express in Hale,
and the flower renders any locus tower. Plugins become
**pure upside** — they exist when a domain's polish bar
justifies the investment, not because the user would
otherwise be stuck looking at a blank pane.

#### Three specialization shapes

A domain plugin scene picks one of three shapes:

1. **Replace.** Ship a scene that takes over the viz pane
   when active. Right for domains with strong non-flower
   visual conventions:
   - `brain3.topology` as a directed DAG of supervisors /
     operators / gateways with idea bindings
   - `<demo-app>.strategy` as a strategy tower with PnL overlay
     and risk-gate flow indicators
   - Network protocol viz as a packet-flow diagram
   The plugin implements the full `VizScene` interface; the
   user (or agent via `viz.set_scene` proposal) switches to
   it explicitly.

2. **Decorate.** Don't replace the flower — attach
   domain-specific glyphs / colors / labels / animations to
   flower petals via scene-channel proposals. The plugin is
   a small locus that listens for domain events
   (`subscribe BrainTaskStarted as on_task_start`) and emits
   `viz.scene.glyph(petal_id, glyph)` /
   `viz.scene.tint(petal_id, color)` proposals. The flower
   scene exposes a decoration channel set for plugins to
   write to. Same flower, richer per-petal info. Cheaper
   to build, preserves iris's brand visual.

3. **Layer.** Hybrid — flower as the structural substrate,
   domain overlay rendered on top in a separate pass. Right
   for "I want the flower AND a domain-specific projection
   showing through it." Probably needs a small interface
   addition (`fn overlay_render(s)` after `render(s)`);
   deferred until a workload demands.

Most v0 specialization should reach for **decorate** first
(cheapest, preserves the iris brand), then **replace** if
the domain genuinely wants a different shape, with **layer**
the rare hybrid case.

#### Why viz is still genuinely pluggable

Rendering structured information into 3D is domain-specific
when the domain has conventions worth honoring (network
arrows, candlestick charts, force-directed graphs). iris
stays domain-agnostic at the substrate (3D primitives,
camera, modes); each domain owns its viz. The flower
default is the substrate-honest baseline; plugins add the
domain-specific polish.

### `Pane`

Lives in [`pane.hl`](./pane.hl). Currently four built-in
panes (shell, source, viz, agent). New panes are possible
(VISION §13 notes "fifth pane" as an open question).

```hale
interface Pane {
    fn focus_gain();
    fn focus_lose();
    fn render(s: RenderSurface);
    fn name() -> String;
    fn on_key(k: KeyEvent);
}
```

### Tools — restructured

Earlier sketches had a `Tool` interface for agent-callable
actions. That model was tied to direct tool-call dispatch.
The read/write split in VISION §6 / AGENT_PROTOCOL.md replaces
it:

- **Read tools** are MCP server methods on `McpServer`, not
  separate `Tool` loci. Adding a read tool means adding a
  method to `McpServer` and a `tools/list` entry. See
  AGENT_PROTOCOL.md §1.
- **Write actions** are proposals dispatched to `ide.cmd.*`
  channels. New writes mean new channels in PLAN.md §3.3 and
  new entries in `dispatch_to_channel`. No `Tool` interface
  needed.

The `Tool` interface from earlier PLUGINS drafts is **retired**.
If a future workload wants user-installable read-tool plugins,
we'll grow the MCP server's tool registry to be plugin-shaped
then.

## What a plugin manifest would look like (sketch, deferred)

Preserved here for the day a workload demands it. Not
implemented.

```toml
# iris/plugins.toml
[backends]
echo      = { path = "agent_backend_echo",      enabled = true }
anthropic = { path = "agent_backend_anthropic", enabled = true }
brain3    = { path = "agent_backend_brain3",    enabled = false }

[viz_scenes]
# Built-in scenes always linked. Plugin scenes register here.
brain3_topology = { path = "lib/scenes/brain3_topology", enabled = true, requires = ["brain3"] }
<demo-app>_strategy = { path = "lib/scenes/<demo-app>_strategy", enabled = false }

[panes]
# Built-in panes always linked; only extra panes register here.
```

Per-workspace config picks the active backend from the
compiled-in set:

```json
{
  "agent": {
    "backend": "anthropic",
    "backend_config": {
      "anthropic": { "model": "claude-opus-4-7", ... }
    }
  }
}
```

## When this comes back

Promote from deferred to active when at least one of:

1. A third-party author has a concrete plugin they want to
   ship and the in-tree hardcode is the blocker.
2. iris ships >3 first-party backends and the conditional-
   compile shape becomes unwieldy.
3. VISION's first-domain audiences (multi-agent
   orchestration, financial, org modeling) start producing
   domain-specific pane variants that benefit from
   plugin-shape distribution.

Until one of those, the load-bearing interfaces stay where
they are (in code) and the manifest stays in this document.
