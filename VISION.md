# VISION

The strategic / conceptual frame for iris. Companion to `PLAN.md`
(which is the implementation roadmap) and `README.md` (which is
the orientation). Where PLAN.md says *what to build and in what
order*, this file says *what iris is, who it's for, and what
makes it defensible*. Implementation decisions should check
against this; deviations need an explicit `FRICTION.md` entry.

---

## 1. What iris is

**iris is a modeling cockpit for structured information.** Not
an IDE. Not a text editor. Not a chat UI. Not a notebook. Not a
whiteboard.

The cockpit is built around a **trinity** of always-visible
surfaces — **viz** (the lotus flower; the cinematic substrate
where the model lives), **source** (the file readout; the
model's textual form), and **agent** (the steering surface;
chat + narration). Each updates in response to the other two;
together they're the modeling loop. A fourth pane, **shell**,
sits as a toggleable drawer beneath the viz pane — present and
mechanically equivalent to the trinity, but visually summoned
on demand because it's an escape hatch, not part of the
modeling loop. See §13 for the layout commitment and the
"why not quad?" reasoning.

The framing matters because "IDE" prejudges the audience
(developers), the artifact (code), and the workflow (typing
keystrokes into source files). iris was built around primitives
— recursive hypergraph, locus tower, vertical-only flow,
PermissionGate, two-channel bus — that apply to **any** form of
structured information. Code is one realization. Strategy docs,
research papers, agent topologies, org charts, financial
strategies, supply chains, decision trees are others. The
substrate is the same; the visible artifact varies by domain.

iris is the cockpit through which a human and an agent
concurrently work with the structure of whatever they're
modeling.

The modeling notation is **Hale itself**. iris uses Hale's
recursive locus tower as the universal description language for
any domain. For Hale source projects, the codebase IS the
model (iris is an Hale IDE out of the box). For non-Hale
domains, the agent writes Hale on the fly that models the
domain's structure, and iris visualizes that Hale. One
notation, one default visualization, every domain. This is what
makes The Design's substrate-invariance claim operational
instead of aspirational — see §4.

## 2. The single load-bearing architectural commitment

**Two independent input channels on the bus.**

- `ide.input.<pane>.*` carries user keystrokes, focus-routed.
- `ide.cmd.<pane>.*` carries programmatic commands, always-on.

A pane subscribes to both and doesn't know which is which. The
agent drives panes the user isn't focused on without stealing
focus, because the channels never collide. Every other design
decision is downstream of this one.

This is structurally novel. Current AI-IDE tools (Cursor, Claude
Code, Copilot, Windsurf, Zed AI, Cody) are **turn-taking**: the
user types or the agent types, never both contending for the
same surface. iris's commitment is **concurrent human+agent work
without contention**. The bus-routing is structurally why iris
can do it and the turn-taking tools cannot.

The agent can be researching in the shell pane, navigating in
the viz pane, and queueing edits for the source pane — all while
the user is typing in the source pane. The user never feels the
agent's presence as interruption.

## 3. PermissionGate as structural enforcement

Per-pane permission gating is a first-class locus
(`PermissionGate`), not a polite-agent convention or a popup. The
user locks the agent out of arbitrary panes at any time; the gate
is enforced at the bus level — `ide.cmd.<pane>.*` traffic that
the gate refuses doesn't reach the pane. Not "an agent that asks
permission nicely"; the gate is mechanical.

This matters because:

- **Trust is structural, not behavioral.** The user doesn't need
  to trust the agent's discretion. The gate's state is the
  contract.
- **Audit is clean.** Every gate transition (lock, unlock, allow,
  refuse) is a bus event. The history of who-could-do-what-when
  is the bus log.
- **Composition is uniform.** External agents (Claude Code,
  Cursor) calling iris through MCP face the same gate. The
  internal agent in iris's agent-pane faces the same gate.
  No two permission models.

## 4. What the viz pane renders

**The locus structure of whatever you're modeling, rendered as
a lotus flower by default.**

The mechanism is uniform across domains. The viz pane hosts a
pluggable `VizScene`; iris ships **`lotus.flower`** as the
default scene. The flower parses Hale source from the
workspace (or from the agent's in-session writes), maps each
locus to a petal, and renders the tower as concentric petals
radiating from the root — projection class drives color family,
lifecycle drives animation (birth → petal grows in; mutation →
shimmer; dissolve → wilt), bus traffic pulses edges between
petals. The agent drives spotlighting via scene-channel
proposals (`viz.scene.spotlight`, `viz.scene.pulse`,
`viz.scene.draw_edge`, `viz.scene.narrate`).

Hale is iris's universal modeling notation. The two cases:

- **Hale source projects.** The codebase IS the model. The
  flower renders the project's actual locus tower live as the
  user edits. iris is an Hale IDE out of the box.
- **Non-Hale domains.** The agent writes Hale on the fly
  that models the domain's structure — each locus becomes a
  petal. The flower renders the agent's model. Persisted in
  `<workspace>/.iris/models/*.hl`; the user reviews the `.hl`
  in the source pane while watching the flower bloom.

| Domain | What the agent writes |
|---|---|
| `.hl` source project | (no agent modeling — the source IS the model) |
| Multi-agent system | `supervisor.hl`, `tools.hl`, `permissions.hl` — loci for each agent / tool / gate |
| Trading strategy | `strategy.hl`, `risk.hl` — strategy tower, risk gates as loci, capacity slots for position state |
| Research paper | `claims.hl` — claim / evidence / citation as types and loci; argument flow as bus topics |
| Org chart | `org.hl` — divisions / teams / roles as loci; escalation paths as F.20 interfaces |
| Supply chain | `flow.hl` — sources / processors / sinks as loci; bottlenecks as capacity slots |
| Meeting transcript | `decisions.hl` — decisions / owners / blockers as loci, follow-ups as bus topics |

The substrate claim is **the recursive hypergraph is the
structure of structured information wherever it has been
structured.** Hale's discipline is the modeling language
that names that structure; the flower is the rendering that
makes it navigable. Code makes the structure explicit;
documents make it implicit; the agent's Hale model makes the
implicit explicit; the flower makes the explicit beautiful.

**Domain specialization happens at the scene layer, not the
mechanism layer.** Three shapes a domain plugin can take:

1. **Replace.** Ship a scene that takes over the viz pane
   (`brain3.topology` as a directed DAG, `<demo-app>.strategy` as
   strategy towers with PnL overlays). Right when the domain
   has strong visual conventions that aren't flower-shaped.
2. **Decorate.** Attach domain-specific glyphs / colors /
   labels / animations to flower petals via scene-channel
   proposals. Same flower, richer per-petal info.
3. **Layer.** Flower as substrate, domain overlay on top.

The flower raises the floor: every domain works decently
without any plugin, because the agent can always write
Hale and the flower can always render it. Plugins are pure
upside — they exist when a domain's polish bar justifies the
investment, not because the user would otherwise be stuck
looking at a blank pane.

## 5. Distribution mechanism — iris is summoned, not switched-to

**iris ships as a binary. The developer's existing agent
(Claude Code, Cursor, Windsurf, etc.) summons it when the task
warrants.** Developers don't switch from VSCode to iris;
their agent gains new tools when iris is installed.

The mechanism, in order:

1. Developer installs iris (one command — homebrew tap,
   `curl|sh`, `cargo install`).
2. iris is on PATH. `iris --help` is rich, agent-readable,
   structured.
3. Developer working in their existing agent interface asks
   the agent something. Somewhere in the agent's reasoning,
   it considers whether iris fits the task.
4. The agent reads `iris --help` (cold-intro brief: what iris
   is, what panes it has, what tools it exposes, when to use
   it, when NOT to use it).
5. If the task fits, the agent calls `iris.launch(workspace=$cwd)`
   via MCP. iris opens.
6. The agent now has iris's MCP read tools available
   (`pane.read`, `viz.snapshot`, `viz.find`, `bus.recent`,
   `permission.state`). It uses them to orient.
7. For mutations, the agent emits structured-output proposals
   onto `ide.cmd.<pane>.*` via its response stream. iris's
   mechanical outer loop reads them, validates against
   PermissionGate, dispatches.
8. User and agent work concurrently in iris until the task is
   done. Agent releases iris when finished; user closes the
   window when ready.

**The strategic asymmetry**: developers don't have to be sold
on iris. Their agent — already in their workflow — discovers
iris when iris is installed, reads iris's brief, and recommends
iris when the task fits. Self-qualifying audience. Zero
switching cost. The agent is the marketing channel.

This sidesteps the historical adoption barrier for IDEs (the
switching cost from VSCode). iris doesn't compete with VSCode
for the primary cockpit. iris is summoned for the kinds of
tasks where its structural-cockpit shape is uniquely valuable;
the developer keeps VSCode (or whatever) for everything else.

## 6. The read/write split

**Reads are in-band MCP tools. Writes are out-of-band
structured-output proposals processed by a mechanical outer
loop.**

Lesson from brain3: agents calling MCP tools to mutate state is
**nondeterministic**. The agent might call the tool, might not,
might call it twice, might call it with subtly different args.
For reads this is fine — the agent decides what to look at, and
looking-or-not-looking doesn't change state. For writes it's
structurally broken — you can't have state mutations gated by
model-decision-to-call when the model is nondeterministic about
calling.

iris's MCP server surface (read side):

- `pane.read(pane_id, range?)` — read pane content
- `pane.focus_state()` — which pane has focus
- `viz.snapshot()` — current locus structure
- `viz.find(query)` — locate a locus / topic / contract
- `bus.recent(subject, n)` — last N events on a subject
- `permission.state()` — current gate matrix

iris's structured-output proposal protocol (write side):

```
iris.proposal {
  channel: "ide.cmd.source.write",
  payload: { file: "adapter.hl", insert_at_line: 42, content: "..." },
  reason: "adding null check on conn_fd"
}
```

The agent emits these in its response stream. iris's mechanical
outer loop parses, validates against `PermissionGate.allow(channel,
payload)`, dispatches if permitted, returns the result via the next
read cycle. The agent never *calls* a write tool — it *proposes*
onto the bus via structured output and the framework decides.

The two-channel bus architecture **is** this read/write split,
already designed into iris before the brain3 lesson surfaced.
`ide.cmd.<pane>.*` is the deterministic-execution channel for
agent-proposed mutations. Determinism lives in the outer loop;
intent lives in the agent's structured output. The boundary is
mechanical.

This pattern generalizes beyond iris — it's the right shape for
any agent-to-system protocol where state matters. Reads are an
in-band action space (model can decide); writes are an
out-of-band protocol (mechanical loop dispatches). Most agent
frameworks conflate these and inherit nondeterminism. iris does
not.

## 7. First-domain audiences

The substrate claim ("locus structure is the shape of any
structured information") is universal. The launch positioning is
not. We lead with audiences who already feel the pain of
implicit structure failing — who will recognize iris's
contribution immediately:

1. **Multi-agent orchestration teams.** Building agent systems
   where supervisor trees, tool registries, permission matrices,
   and escalation paths matter. Currently building these in
   Python with custom DSLs (LangGraph, CrewAI, Sierra, internal
   frameworks). The locus structure of an agent topology is
   exactly Hale's locus tower. The agent models the topology
   in Hale (supervisor / worker / gateway loci, tool registry
   as a `@form(hashmap)`, permission matrix as an F.20
   interface); the lotus flower renders it live. Concurrent
   human+agent work means the modeling agent can iterate on
   the structure while the developer adjusts a different part.
   This is iris's strongest first-domain claim: the audience
   (people building agent systems) is growing fast; the
   substrate fit is 1:1; the wedge (concurrent human+agent +
   PermissionGate) is exactly the discipline agent-system
   builders need.

2. **Financial systems modeling.** Trading strategies, risk
   hierarchies, market structure, online-learning topologies.
   Substrate fit is direct because financial systems have
   formal flow rules and capacity bounds. <demo-app> is the
   production-shape proving ground. Viz pane renders strategy
   towers, risk gates, data flow, PnL attribution.

3. **Organizational modeling.** brain3's territory. Executable
   org charts, division supervision, role contracts, escalation
   flow. The "executable org chart" pitch is the locus tower of
   an org made navigable. Viz pane renders the hierarchy live;
   the agent can model org changes structurally before they
   happen.

These three share a property: **the system being modeled has
real structural rules**. Loci, contracts, vertical flow aren't
analogies — they're the actual substrate. The viz pane has
something concrete to render. The agent has tools that match
the domain.

**Broader domains are forward content, not launch positioning.**
Documents, research papers, strategy docs, supply chains,
meeting transcripts all have extractable locus structure (per
§4 above) and iris will work for them — but the launch leads
with the three above because their audiences are ready to value
the structural-cockpit shape *today*. Broader applicability
earns itself as iris proves out in the first domains.

## 8. What iris is not

Negative space matters as much as positive claims:

- **iris is not "the new VSCode."** No ambition to replace
  the developer's primary cockpit. iris is summoned for
  specific tasks; the developer's existing tools stay.
- **iris is not a chat UI.** The agent pane is one of four
  panes, not the primary surface. The agent works in the
  structure, not in conversation.
- **iris is not a notebook.** No cell-based execution model.
  No interleaved markdown+code+output. The four panes are
  structural roles, not authoring conventions.
- **iris is not a whiteboard.** Not free-form 2D space.
  Structure is constrained to the locus tower's vertical
  flow; the viz pane is hierarchical, not spatial-arbitrary.
- **iris is not a verification tool.** No Z3 SMT. Hale's
  closure tests + PermissionGate + structural typing handle
  the discipline at the runtime layer. iris surfaces them
  but doesn't add a verification surface beyond what the
  language already commits to.
- **iris is not "for everyone."** Keyboard-first, raylib-
  rendered, locus-structured. The audience is developers,
  modelers, analysts, researchers — people who work with
  structured information and want concurrent agent help.
  Mouse-first / document-first / artist-first audiences are
  served better elsewhere.

These omissions are intentional. The claim is not "iris does
more than other tools." The claim is "iris does the specific
thing — concurrent human+agent work on locus structure — that
no other tool does."

## 9. The wedge stated plainly

Whatever domain framing, the differentiator is unchanged:

> **iris is the first cockpit for concurrent human+agent work
> on structured information, with structural permission gating
> and live locus-structure visualization. Nothing else ships
> this combination.**

If the pitch is "modeling tool for any domain" the
differentiator dilutes — people compare iris to Notion and
Notion wins on document features iris doesn't have. If the
pitch is "concurrent human+agent + structural cockpit + live
extraction" the differentiator is the lead. Nothing competes
on that axis.

The language (Hale) is the substrate that makes the
combination possible. The combination is the wedge. Lead with
the wedge; the language follows.

## 10. Position within alpha-conjecture

iris is one of several substrate-operationalizations of The
Design:

- **brain3** — typed-edge AI-orchestration platform.
  Operationalizes The Design at the software-coordination
  substrate. In production at the firm.
- **<demo-app>** — multi-binary message-driven finance system.
  Operationalizes The Design at the financial-substrate.
  Replacing internal infra (`grease` and `magus`).
- **Hale language + stdlib + pond + hale-js** — the
  language substrate The Design lives in at the
  programming-language layer. Two runtimes (native + browser)
  with substrate-invariance preserved across both.
- **iris** — the modeling cockpit through which The Design's
  locus-tower structure is *exposed and worked on
  concurrently by humans and agents at any substrate*.

iris is **not** a fourth domain-application alongside brain3
and <demo-app>. iris is the **cockpit** through which any
domain-application's locus structure becomes navigable. brain3
runs on iris (you model an agent organization through iris's
panes). <demo-app> runs on iris (you model a trading strategy
through iris's panes). A research workflow runs on iris (you
model the paper's claim structure through iris's panes).

This makes iris structurally the "second-tier" piece — it
applies to whatever first-tier substrate is being modeled.
Same as Hale applies to any system being built; iris applies
to any system being modeled.

The five papers (capacity_allocation, mad_saturation,
design_frame_projection, closed_horizon_recursion,
motion_rooted_language) formalize the underlying claims that
make iris's substrate-invariance commitment defensible. iris
itself does not need to publish — its load-bearing role is the
*operationalization*, not the formal proof. But the papers
ground why "extract locus structure from any structured
information" is a real claim rather than a marketing line.

## 11. Success criteria

iris succeeds if and only if:

1. **The two-channel bus + PermissionGate combination is
   structurally enforced**, not advisory. A locus that tries to
   write to a gated pane is mechanically refused. (Test:
   adversarial agent prompts that try to bypass the gate fail
   at the bus layer.)
2. **The agent models at least one non-Hale domain in
   Hale; the flower renders it; a domain expert recognizes
   the structure.** First target: agent topology for a
   multi-agent system (agent writes `supervisor.hl` /
   `tools.hl` / `permissions.hl`; flower blooms the topology).
   Second: research-paper claim graph. (Test: the domain
   expert watches the agent model their domain in Hale,
   reads the `.hl` in the source pane, watches the flower
   render the model, and reports that the structure matches
   their mental model. The proof is the agent's modeling
   skill plus the flower's rendering — no domain-specific
   plugin required.)
3. **External agents can summon iris via MCP and operate
   concurrently with the user.** Claude Code reads `iris --help`,
   launches iris, drives one pane while the user drives
   another, all in the same workflow without alt-tab
   interruption. (Test: a Claude Code session and a user
   complete a real task in iris with both contributing
   concurrently.)
4. **Mutations come through structured output, not MCP
   tool-calls.** The agent never calls a write tool; it emits
   proposals that the outer loop validates and dispatches.
   (Test: a stress harness running the agent 100 times on the
   same input produces deterministic mutation sequences.)

Without these, iris is a fancy multi-pane editor with raylib
rendering. With them, iris is the structural cockpit the wedge
calls for.

## 12. What this commits to over time

- **iris stays narrow on its wedge.** Concurrent human+agent +
  structural cockpit + live extraction. New panes / features /
  capabilities are evaluated against whether they sharpen the
  wedge or dilute it.
- **iris stays summoned, not switched-to.** Distribution
  through agent-mediated discovery. No marketing surface that
  asks developers to abandon their existing tools.
- **iris stays a cockpit, not an editor.** If a feature
  request makes iris better at being a primary editor at the
  cost of being worse at being a structural cockpit, the
  feature is rejected and logged in `FRICTION.md`.
- **The first-domain audiences (agent orchestration,
  financial, org modeling) drive feature priority.** Broader
  applicability is forward content; the wedge sharpens first.
- **Substrate-invariance commitments are tested empirically
  before the broader-domain claims are made publicly.** If
  iris can't render a research paper's locus structure in a
  way the author recognizes, "any structured information" is
  premature.

The work compounds when these commitments hold. The work
fragments when they don't.

## 13. Open questions deferred to working sessions

- **Pane composition.** **Ruled (2026-05-23):** the cockpit
  is a **trinity** of always-visible panes — **viz**,
  **source**, **agent** — with **shell** as a toggleable
  drawer beneath the viz pane (`` Ctrl+` ``). Default layout:
  viz takes the left ~2/3 of the window full-height; source
  and agent stack in the right ~1/3 column; shell is
  collapsed by default and slides up from the bottom of the
  viz column when summoned. The drawer-in-viz-column shape
  localizes the shell↔viz tradeoff to one column — source
  and agent stay stable when shell toggles.

  **Why trinity, not quad.** Post-flower-pivot, three panes
  are load-bearing for the modeling loop: viz carries the
  cinematic state, source carries the model, agent steers.
  Each updates in response to the other two. Shell is a
  powerful escape hatch (`hale test`, grep, vim, htop) but
  it isn't part of the modeling loop — it doesn't update in
  response to the other panes and it doesn't carry
  structural state the others read. Giving it always-on
  visual real estate shrinks the three load-bearing panes
  for a pane that's most often unused. For non-Hale
  modeling (one of the headline use cases), shell is dead
  weight entirely.

  **Why a drawer, not summoning.** A summon-from-nowhere
  shell would be cheap but disorienting. A drawer that lives
  *inside the viz column* keeps the flower visible even when
  shell is open (collapsed flower thumbnail at the top of
  the column), so the user retains ambient "what's the agent
  doing to my model" signal while doing terminal work.
  Cockpit-shaped: primary instrument stays in peripheral
  view when you're looking at the secondary panel.

  **What was rejected.** A tabbed-window restructure (one
  pane visible at a time, browser-style tabs to switch) was
  considered and rejected — it kills the concurrent-work
  signal per §2 (the agent can still drive hidden tabs on
  `ide.cmd.*`, but the user loses the live "I'm typing in
  source while the agent is updating viz" wedge that
  distinguishes iris from turn-taking tools). The original
  4-pane-equal layout was also rejected: post-flower, viz
  deserves cockpit center-of-mass and shell doesn't earn its
  quadrant.

  **What's still flexible.** Visual density within the
  trinity is addressed via resizable dividers, a maximize
  keybind (`Ctrl+M`) that collapses non-focused panes to a
  thin status strip preserving activity indicators, and
  layout presets per task shape (`Ctrl+Shift+1..4`). Tabs
  *within* a single pane (multiple source files, multiple
  shell sessions) remain post-v0.1 polish and don't touch
  the wedge. Architecturally nothing changes — ShellPane
  remains a peer locus on the bus, PermissionGate enforces
  on shell channels identically, `shell.exec` proposals
  unchanged. Only visual default changes.
- **Multi-user iris.** Two humans + agent in the same iris
  session, each focus-routed independently. Possible with the
  bus architecture; not implemented; not needed until the
  workload surfaces.
- **Persistence across sessions.** Workspace state in
  `<workspace>/.iris/` per the README. The right shape for
  long-running modeling work (resume a research-paper modeling
  session after a week) is forward content.
- **Cross-iris federation.** Multiple iris instances on
  different machines coordinating via `TransportBridge` over
  WebSocket. The hale-js protocol exists; iris-to-iris is
  forward.
- **Plugin surface.** PLUGINS.md sketches the shape;
  implementation gated on a workload that needs it.

These are tracked as forward content. The current build holds
to the four-pane shape with single-user / single-iris /
in-memory state until a real workload forces a different
choice.

---

## Cross-references

- `PLAN.md` — implementation phasing, architectural details,
  per-phase deliverables
- `README.md` — orientation, install, workflow
- `PLUGINS.md` — plugin-surface sketch (deferred)
- `FRICTION.md` — running log of design tensions and
  resolutions
- `~/code/hale/AGENTS.md` — Hale language conventions
- `~/code/hale-lang/pond/README.md` — pond contrib library
  catalog
- `~/notes/alpha-conjecture/theory/the_design.md` — the
  substrate-invariant recursion operator iris operationalizes

iris's claim is not "a new programming environment." It is
"the first cockpit for concurrent human+agent work on the
structural shape of any structured information, with
structural enforcement of the boundaries between them." That
sentence is what every implementation decision checks against.
