# Using iris

The user-facing surface. What you see, what you do, what you
need to learn. Companion to [`VISION.md`](./VISION.md) (the
strategic frame), [`PLAN.md`](./PLAN.md) (the implementation
roadmap), and [`AGENT_PROTOCOL.md`](./AGENT_PROTOCOL.md) (the
agent integration contract). Where those describe iris from
the inside, this file describes it from the outside.

Aspirational where the code isn't there yet — flagged
explicitly in the "v0 vs. later" section. Don't promise
features in this file that PLAN doesn't deliver in a named
phase.

---

## What iris looks like

`iris .` opens a window with a **trinity layout** — three
always-visible panes (viz, source, agent) plus shell as a
toggleable drawer beneath the viz pane.

**Default (shell collapsed):**

```
┌──────────────────────────┬─────────────┐
│                          │   source    │
│                          │ [file view] │
│           viz            ├─────────────┤
│  [lotus flower scene]    │   agent     │
│                          │ [chat +     │
│                          │  input]     │
└──────────────────────────┴─────────────┘
```

**Shell drawer open (`` Ctrl+` ``):**

```
┌──────────────────────────┬─────────────┐
│           viz            │   source    │
│  [lotus flower scene]    │             │
│                          ├─────────────┤
├──────────────────────────┤   agent     │
│   shell                  │             │
│   [your $SHELL — PTY]    │             │
└──────────────────────────┴─────────────┘
```

Why this shape: three of the four panes are part of the
modeling loop — viz carries the cinematic state, source
carries the model text, agent steers. They earn always-on
real estate. Shell is a powerful escape hatch
(`hale test`, grep, vim, htop) but it's not part of the
loop; it slides up from under the viz when needed and back
down when not. The flower stays visible even when shell is
open — its thumbnail strip remains at the top of the column,
so the user keeps ambient "what's the agent doing to my
model" signal while doing terminal work. See VISION.md §13
for the full layout commitment.

Cues to look for:

- **Pane border** — bright when the pane has focus; dim when
  it doesn't. The focused pane is where your keystrokes go.
- **Lock icon** in a pane's corner — present when the agent
  is locked out of writing (or reading) that pane. Set by
  you, persists across sessions.
- **Toast** (corner overlay, top-right) — transient messages
  like *"agent tried open_file → source pane locked."*
  Fade in 2 seconds.
- **Status line** (planned, bottom of window) — a single
  thin strip with five segments, separated by dim middle-
  dots:

  ```
  ◇ hale-lang/iris  ·  anthropic › hale.locus_tower  ·  ▸agent · ◉B  ·  ◯◯◯◯  ·  ⠿
  ```

  Reading left to right:

  - `◇ <workspace>` — workspace name, truncated to last two
    path segments. `◇` is the workspace marker (accent
    color).
  - `<backend> › <scene>` — active AgentBackend, then
    active VizScene. The `›` is a dim chevron meaning "and
    inside that, …".
  - `▸<focus> · ◉<mode>` — focused pane name, then viz mode
    (`B` / `H` / `R` for bulk / harmonic / resolution). The
    `▸` is an accent arrow; `◉` is the filled mode glyph.
  - `◯◯◯◯` — per-pane lock indicators, one circle per pane
    in `shell source viz agent` order. `◯` = unlocked,
    `●` = locked (write-blocked from agent). Colored to
    draw attention when something is locked.
  - `⠿` — agent activity. Animated braille spinner
    (`⠿⠷⠯⠟⠻⠽⠾`) while the agent is streaming; blank when
    idle.

  All glyphs are common Unicode shipped with raylib's
  bundled monospace font. ASCII fallback is straightforward
  if the font ever changes.

## What's in each pane

The three trinity panes are described first because they're
the modeling loop. Shell is described last because it's the
toggleable escape hatch.

### viz — the structural shape (lotus flower by default)

The cockpit's centerpiece. A 3D rendering pane that hosts a
**scene** — a pluggable component that defines what to
render. iris ships **`lotus.flower`** as the universal
default: a beautiful animated lotus where each Hale locus
is a petal, lifecycle drives animation (birth → grow,
mutation → shimmer, dissolve → wilt), and the agent drives
spotlighting / pulsing / edge overlays via scene proposals.

The flower works in two cases without per-domain code:

- **Hale source projects** — the codebase IS the model.
  The flower renders the project's actual locus tower live as
  you edit. iris is an Hale IDE out of the box.
- **Non-Hale domains** — the agent writes Hale that
  *models* the domain (see "Modeling a non-Hale domain"
  below); the flower renders the agent's model.

One notation (Hale), one default visualization (the
flower), every domain. Domain plugins exist as polish — not
necessity — and can specialize three ways: **replace** (ship
a scene that takes over the pane, e.g. `brain3.topology` as a
directed DAG), **decorate** (attach glyphs / colors / labels
to flower petals via scene channels), or **layer** (flower as
substrate, domain overlay on top).

Planned plugin scenes (post v0.1, when polish bar justifies):

- `brain3.topology` — supervisor / operator / gateway DAG
  with idea bindings, routing rules, active tasks
- `<demo-app>.strategy` — strategy tower, risk gates, data
  flow, PnL attribution
- `org.hierarchy`, `paper.claim_graph`, etc.

For v0.1 every domain runs on the flower. That's by design —
the flower raises the floor so plugins are pure upside, not
prerequisites.

Substrate that's always universal (any scene gets these):

- **Three modes** (`1` / `2` / `3` keys):
  - **bulk** — every node rendered, instanced
  - **harmonic** — nodes grouped by type / class, tinted
  - **resolution** — camera framed on one node, others dimmed
- **Camera** — WASD fly, mouse drag orbit, scroll zoom.
- **Click a node** — delegated to the active scene. The
  default `lotus.flower` navigates the source pane to the
  clicked locus's source location (or to the agent's model
  file for non-Hale domains); domain plugin scenes define
  their own click semantics.

Scene-specific operations (each scene defines its own — the
agent discovers them via `viz.scene_info()`). The default
`lotus.flower` exposes the cinematic surface the agent uses
to walk you through a model:

- `viz.scene.spotlight(ids)` — glow these petals, dim
  others
- `viz.scene.pulse(id, color)` — flash a petal in a color
- `viz.scene.draw_edge(a, b)` — draw a relationship line
- `viz.scene.narrate(id, text)` — caption overlay on a
  petal

You'd use it for: seeing the *shape* of what you're working
on, navigating to specific structural points, watching the
shape change as you edit Hale code (or as the agent writes
Hale modeling your domain).

### source — what you're modeling (readout-only)

A file **viewer**, not an editor. iris is aware of which
file you're showing and watches it via fsnotify; when the
file changes — whether from the agent writing to it via a
`source.write` proposal, or from you editing it in your
external editor (vim, VSCode, whatever) — iris re-reads and
re-displays automatically. Optionally highlights what just
changed.

This is deliberate. iris doesn't compete with editors on
editing. You keep your editor open in another window for
keystroke-level work; iris is the cockpit that shows you
what's happening to the file (whether it's you or the agent
making the change) and what its structure looks like (viz
pane).

For Hale projects, this is the source code of what you're
modeling. For non-Hale domains, this is where you read the
`.hl` model the agent wrote in `<workspace>/.iris/models/` —
your audit trail for the structural choices the agent made.

You'd use it for: watching the file change as you (or the
agent) edit it; reading specific lines / ranges the agent
points you at; understanding what the agent just did;
verifying the agent's domain model.

### agent — your collaborator

A chat-style interface with the LLM that's working alongside
you. Your input goes in a line at the bottom; the agent's
response chunks stream above.

**The agent does more than chat.** As it works, it emits
*proposals* — structured actions it wants iris to perform on
other panes. Open a file. Run a command. Add a node to the
viz scene. Each proposal flows through iris's mechanical
permission gate. Approved proposals apply; denied ones
surface as toasts.

In the agent pane scrollback you'll see:

```
you: rename submit_user_input to handle_prompt
agent: I'll find usages first...
[agent reads source via pane.read]
agent: Found 3 occurrences in agent_harness.hl. Renaming.
[agent proposed: source.write at agent_harness.hl:17 → applied]
[agent proposed: source.write at agent_harness.hl:23 → applied]
[agent proposed: source.write at main.hl:142 → applied]
agent: Done. 3 occurrences renamed.
```

You'd use it for: asking the agent to do things, watching it
narrate its work, reviewing what it proposes, course-
correcting.

### shell — toggleable escape hatch

A PTY-backed terminal running your `$SHELL` (bash, fish,
zsh, etc.), accessible as a drawer that slides up from the
bottom of the viz column. Output is ANSI-rendered (color,
cursor moves, scroll regions); `vim`, `htop`, interactive
REPLs all work. Press `` Ctrl+` `` to toggle, or `Ctrl+1`
to focus (auto-opens if collapsed).

When the drawer is open, the viz pane shrinks vertically
above it — the flower stays visible (just shorter) so
you keep ambient model-state signal while doing terminal
work. Drag the handle between viz and shell to resize.
Source and agent on the right column are unaffected.

Architecturally, shell is a peer pane — same bus channels,
same PermissionGate enforcement, same proposal dispatch.
When the drawer is closed, the PTY is still running and
scrollback is still accumulating; only its render is gated
by the drawer state.

You'd use it for: `hale test`, `hale build`, ad-hoc
commands, inspecting output, anything that needs an
interactive shell. If you find yourself living in shell
constantly, drag the divider so it dominates the column,
or pin the `shell-up` layout preset (`Ctrl+Shift+4`).

## What you do — keyboard-first

iris is built around keyboard navigation. Mouse is secondary
(orbit + entity click in the viz pane, text selection
elsewhere).

### Global keys (work from any pane)

| Key | Action |
|---|---|
| `Ctrl+Tab` | Cycle focus forward through visible panes (skips shell when drawer is collapsed) |
| `Ctrl+Shift+Tab` | Cycle focus backward |
| `Ctrl+1` | Focus shell (auto-opens drawer if collapsed) |
| `Ctrl+2` / `Ctrl+3` / `Ctrl+4` | Jump focus to source / viz / agent |
| `` Ctrl+` `` | Toggle shell drawer beneath the viz pane (last-used height) |
| `Ctrl+M` | Maximize focused trinity pane (other two collapse to a thin status strip with activity indicators); press again to restore |
| `Ctrl+Shift+1..4` | Switch layout preset (source-heavy / viz-heavy / agent-heavy / shell-up) |
| `Ctrl+Alt+H/J/K/L` | Nudge pane divider 5% (left / down / up / right) |
| `Ctrl+L` | Toggle agent **write-lock** on the focused pane |
| `Ctrl+Shift+L` | Global lock — all panes write-blocked at once (panic button) |
| `Ctrl+R` | Toggle agent **read-lock** on the focused pane (rare) |
| `Ctrl+Q` | Quit iris |

### Per-pane

**shell** — keystrokes forwarded to the PTY. Your shell sees
them; whatever your shell does is what happens.

**source** — arrow keys / Page Up/Page Down for navigation.
**No editing keystrokes** — source pane is readout-only. Edit
the file in your external editor (or via agent proposal);
fsnotify makes iris reflect the change automatically.

**viz** —

| Key | Action |
|---|---|
| `W` / `A` / `S` / `D` | Camera fly |
| `1` / `2` / `3` | Switch mode (bulk / harmonic / resolution) |
| Mouse drag | Orbit camera |
| Scroll wheel | Zoom |
| Click a node | Focus camera on it (planned: source-pane navigates to corresponding location) |

**agent** —

| Key | Action |
|---|---|
| Any printable character | Append to input buffer |
| `Backspace` | Delete from input buffer |
| `Enter` | Commit the prompt; clear input |
| `Up arrow` | Recall last prompt (planned) |
| `Ctrl+C` (when an agent response is streaming) | Cancel the in-flight turn |

## Vocabulary

Terms that show up in iris docs / output and what they mean.

| Term | What it means |
|---|---|
| **Pane** | A bus-attached surface. iris has four: viz, source, and agent form the always-visible **trinity** (the modeling loop); shell is a **drawer** beneath the viz pane (toggleable via `` Ctrl+` ``). All four are peer panes architecturally — same bus channels, PermissionGate enforcement, proposal dispatch — they differ only in default visibility. |
| **Trinity** | The three always-visible panes (viz, source, agent) that carry the modeling loop. Each updates in response to the other two: agent steers → source carries the model → viz renders the structure. |
| **Drawer** | A pane that defaults to collapsed visual state and slides into view on demand. iris's shell pane is the only drawer at v0.1. Architecturally a peer of the trinity; only the render is gated by the drawer state. |
| **Focus** | Which pane your keystrokes go to. The border indicates it. |
| **Workspace** | The directory iris is operating on. State persists under `<workspace>/.iris/`. |
| **Locus structure** | The structural shape of what's in the source pane. Shown in viz pane. From "locus" — Hale's unit of structure (anything with lifecycle, contracts, or bus participation). |
| **Agent** | The LLM collaborating with you. Reads iris state via **MCP**; writes via **proposals**. |
| **Backend** | The LLM service powering the agent. v0 ships with `echo` (deterministic, no LLM). `anthropic` and `brain3` follow. Switchable in workspace config. |
| **Proposal** | A structured action the agent wants iris to apply (open a file, run a command, add a viz node). Visible inline in the agent pane scrollback as it dispatches or is denied. |
| **Permission gate** | The mechanical guard between agent proposals and pane mutations. Locks are enforced here, not by polite-agent convention. |
| **Lock** | Mark a pane as off-limits to the agent. Read-lock and write-lock are separate bits. Persists across sessions in `.iris/permissions.json`. |
| **MCP** | Model Context Protocol — the JSON-RPC interface other agent tools (Claude Code, Cursor, …) use to read iris's state and summon iris. Optional from the user's perspective — only matters if you're using iris alongside another agent tool. |
| **Summon** | What happens when an external agent calls `iris.launch(workspace=...)` to open iris from elsewhere. Distinct from `iris .` (you launch it from a CLI). |
| **Mode (viz)** | One of `bulk` / `harmonic` / `resolution` — three ways the viz pane renders the same scene at different granularity. |
| **Scene** | The pluggable component that drives the viz pane. iris ships **`lotus.flower`** as the universal default — renders any Hale locus tower as an animated lotus. Domain plugins specialize three ways: replace (their own scene), decorate (glyphs on flower petals), layer (overlay on flower). Switchable via `viz.set_scene` proposal. |
| **Lotus flower** | The default `lotus.flower` scene. Each Hale locus is a petal; projection class drives color family; lifecycle drives animation; agent drives spotlighting via scene channels. Works uniformly for Hale projects (codebase IS the model) and non-Hale domains (agent writes Hale modeling the domain). |
| **Model (non-Hale domain)** | The `.hl` files the agent writes into `<workspace>/.iris/models/` to describe a non-Hale domain's structure. Read by the flower; reviewed by the user in the source pane; persistable / git-trackable / shareable. |
| **Readout** | What the source pane is — a viewer of the file on disk. iris watches the file (fsnotify); the agent writes through proposals that hit disk; the user's external editor writes through normal disk writes; iris reflects everything. iris doesn't edit. |

## A typical session

Walkthrough of an everyday flow. (Most of this lights up in
Phases 3–5; v0.1 will give you a working subset.)

```
$ iris ~/code/myproject
                                  [iris opens; trinity layout —
                                   viz left, source + agent right;
                                   shell drawer collapsed;
                                   agent pane focused]

you: "show me the layout of this project"

  [agent emits viz.snapshot (MCP read) — sees the workspace]
  [agent emits open_file proposals → reads .hl source files]
  [agent emits viz.add_entity proposals → viz pane shows
   the locus tower of the codebase]

you: Ctrl+3                       [focus jumps to viz pane]

you: drag the mouse               [scene rotates; you spot a
                                   suspicious-looking
                                   AgentHarness locus]

you: click the AgentHarness node  [camera focuses it; (planned:
                                   source pane jumps to
                                   agent_harness.hl:17)]

you: Ctrl+4                       [focus back to agent]
you: "what's the cancel() method for?"

  [agent reads agent_harness.hl line 17 via pane.read]
  [agent narrates in chat]

agent: It cancels an in-flight backend call. Currently a
       no-op for EchoBackend (nothing's in flight); will be
       wired to abort the streaming HTTP call once
       AnthropicBackend lands.

you: "add a docstring explaining that"

  [agent proposes source.write at line 17 with a docstring]
  [proposal goes through the gate → applied]
  [source pane shows the change immediately]

you: Ctrl+2                       [focus to source; review]
you: [decide the docstring is wrong; start typing the
      replacement] "// cancel an in-flight backend ..."

  [meanwhile, agent is still streaming its rationale in the
   agent pane scrollback — you don't see it interrupt your
   typing because you have focus and the agent's outputs
   land on a separate channel]

you: [10 minutes later, satisfied]
you: Ctrl+Q                       [iris exits cleanly]

$ iris ~/code/myproject           [next time you launch]
                                  [reopens to the same files
                                   + cursor positions + viz
                                   scene]
```

## Modeling a non-Hale domain

iris isn't only for Hale code. The agent's primary skill
in a non-Hale workspace is **writing Hale that models
your domain**. The flower then renders the agent's model.
You read the agent's `.hl` in the source pane to verify the
structural choices it made.

The flow:

```
$ iris ~/work/org-restructure
                                  [iris opens; source pane
                                   shows whatever's at hand
                                   (org chart CSV, slide deck,
                                   meeting notes); flower
                                   empty]

you: "model our team structure here"

  [agent reads the workspace files via pane.read]
  [agent emits source.write proposals creating
   .iris/models/org.hl — loci for divisions, teams, roles;
   F.20 interfaces for escalation paths; @form(hashmap) for
   the role registry]
  [each write adds petals; flower blooms incrementally]

agent: I modeled this with three top-level divisions, each
       as a chunked-projection locus (you have ~10 teams
       per division). Roles are types (pure data — no
       lifecycle), escalation paths are F.20 interfaces.
       The Finance division has a recognition-projection
       sub-locus for the contractor pool (~200 people, you
       wanted aggregate observation only). Check .iris/
       models/org.hl.

you: Ctrl+2                       [focus source pane]
you: [reads org.hl, notices a missing reporting line]

you: Ctrl+4                       [back to agent]
you: "Engineering reports up through CTO, not COO"

  [agent emits source.write proposal updating the
   relevant interface]
  [flower's escalation edges redraw live]
```

What you get:

- **Persistent model.** The agent's `.hl` files live in
  `<workspace>/.iris/models/`. Commit them; share with
  teammates; iterate over sessions.
- **Auditable choices.** Every modeling decision is in the
  `.hl` source. Want to know why something's structured a
  given way? Read the agent's `.hl` (or check
  `proposals.log` for the reasoning the agent attached to
  each write).
- **Same UX as Hale projects.** Flower, focus, locks,
  permission gate — all identical to the IDE case. The
  agent's writes go through the same proposal pipeline
  your own writes (via `source.write`) would.

What the agent commits to when modeling:

- Each modeled thing with structural commitments becomes a
  **locus** (your team's deploy-pipeline, your strategy's
  risk-gate, your paper's argument).
- Each modeled thing that's pure data becomes a **type**
  (a role, a metric, a citation).
- The locus tower mirrors the domain's natural hierarchy.
- Capacity slots (`@form(vec)` / `@form(hashmap)`) model
  the domain's bounded collections.
- F.20 interfaces model the domain's structural promises
  (an escalation path, a contract, a flow guarantee).

This is what makes substrate-invariance operational:
Hale's discipline is the universal modeling vocabulary,
and the flower renders the vocabulary uniformly. Domain
specialization (richer per-petal glyphs, domain-specific
overlays) is plugin territory; you get a working model on
day one regardless.

## What's different from other tools

Quick comparisons since the framing isn't familiar.

- **Cursor / Claude Code / Copilot / Windsurf** — turn-taking.
  You type OR the agent types; never both at once on the same
  surface. iris: both, on different panes. The agent
  modifies source while you're typing in the agent pane; the
  agent runs shell while you're navigating the viz; nothing
  collides.

- **VSCode** — edits text. iris: works on the *structure* of
  text. The viz pane shows what no editor shows: the
  hierarchical shape of what you're modeling. iris is not
  trying to replace VSCode. Keep VSCode; summon iris when
  the task is structural.

- **Notion / Obsidian** — documents. iris: locus structure of
  any artifact, with an agent that proposes mutations through
  a mechanical gate. The proposals + gate are what no
  document tool has.

- **ChatGPT / Claude.ai** — chat. iris: chat is one of four
  panes, not the primary surface. The structure is primary;
  chat is the steering mechanism.

- **Jupyter / Observable** — notebooks with code cells. iris:
  no cell model. The four panes are structural roles, not
  authoring conventions.

## What's v0.1 vs. later

Honest scoping. v0.1 ships the wedge; everything else
follows.

### v0.1 (the wedge)

- Four panes, keyboard navigation, focus cycling.
- Real LLM responses (Anthropic backend, Phase 4).
- Permission gate with lock/unlock via Ctrl+L; persistence
  in `.iris/permissions.json`.
- Source pane: file readout with fsnotify-driven live
  updates (iris never edits the file; agent writes via
  proposals hit disk; user's external editor + fsnotify
  composes naturally).
- Viz pane: hosts a `VizScene`; default `lotus.flower`
  scene renders any Hale locus tower live as an animated
  lotus (Phase 3).
- Agent-modeled non-Hale domain demo — agent writes
  Hale into `.iris/models/`, flower blooms the model
  (Phase 6). No domain-specific plugin scene required at
  v0.1; the flower covers every domain.
- Dynamic pane prominence — resizable dividers, maximize
  keybind, layout presets (Phase 5).
- MCP server with read tools (Phase 2); external agents
  can summon iris and read state.
- Session restore (Phase 7).
- Real window via raylib (Phase 5, blocked on the `@ffi`
  hale change).

### After v0.1

- Source pane: tabs (multiple files open).
- Source pane: visual diff highlighting (show what just
  changed when fsnotify fires).
- Viz pane: more interaction (click-to-source, drag-to-
  reorganize, multi-selection).
- Themes loaded from config.
- More backends (`brain3` first).
- Cross-iris federation (multiple iris instances syncing).
- Multi-user (two humans + agent in the same session).
- Plugin manifest + third-party panes / backends.
- Domain-specific plugin scenes (`brain3.topology`,
  `<demo-app>.strategy`, etc.) — for domains whose visual
  conventions earn replacing or decorating the flower.

### Out of scope (not coming)

- Replacing VSCode as a primary editor. iris is summoned for
  specific tasks; the developer's existing cockpit stays.
- Mouse-first / document-first / artist-first surfaces.
- Verification surface (Z3, formal proof). Hale's closure
  tests + PermissionGate handle the discipline at the
  runtime layer; iris surfaces it but doesn't add a
  verification surface.
- A cell-based authoring model. The four panes are roles,
  not authoring units.

## When iris is the right tool

You probably want iris when:

- You're modeling something with **real structural rules**
  — a multi-agent system, a trading strategy, an org chart,
  a research-paper claim graph, a supply chain, a process
  flow.
- You want an agent to do **substantive work in parallel**
  with you, not just suggest completions or take turns.
- You want the agent's actions to be **auditable and
  bounded** — every mutation is a proposal in
  `.iris/proposals.log`, every lock is a structural denial.
- The structural shape of what you're modeling is more
  load-bearing than the surface text.

You probably don't want iris when:

- You're writing prose without underlying structure
  (memos, emails, longform articles). Use a text editor or
  word processor.
- You're doing a quick file edit your normal editor handles
  in seconds. Stay there.
- The structure isn't really structure (free-form notes,
  brainstorm dumps). Use a notebook tool.
- You want an agent that types code FOR you in your normal
  editor. That's Cursor / Copilot territory.

## Configuration

Two layers, both JSON for v0:

- **User-global**: `~/.config/iris/config.json`.
- **Workspace-local**: `<workspace>/.iris/config.json` — overrides user.

Config schema details in PLAN.md §3.8. The fields you'll
touch most often:

- `agent.backend` — pick between `echo`, `anthropic`, etc.
- `agent.backend_config.<name>` — backend-specific tuning
  (model, API key env var, etc.).
- `keybinds` — override the default keymap.
- `theme` — colors.
- `mcp.transport` / `mcp.listen_socket` — for external-agent
  summoning.

## Workspace state

Files iris writes under `<workspace>/.iris/`:

- `config.json` — your workspace-local overrides. **Commit
  this** if you want teammates to share the setup.
- `session.json` — last layout / open files / cursor
  positions. Gitignore.
- `permissions.json` — current PermissionGate state. Gitignore
  (locks are typically per-developer judgment).
- `proposals.log` — audit log of every applied + denied
  proposal. Gitignore by default; useful to inspect when
  debugging agent behavior.
- `agent_history.db` — conversation history (backend-managed).
  Gitignore.

iris creates `.iris/` lazily on first state write. Fresh
workspaces don't get cluttered until you actually use them.

---

## Cross-references

- [`VISION.md`](./VISION.md) — why iris exists, what it's
  not, what it commits to over time.
- [`PLAN.md`](./PLAN.md) — engineering roadmap, architecture,
  phases.
- [`AGENT_PROTOCOL.md`](./AGENT_PROTOCOL.md) — formal
  contract for external/internal agent integration. Read if
  you're writing tooling that talks to iris.
- [`FRICTION.md`](./FRICTION.md) — what's been hard,
  workarounds, findings.
- [`README.md`](./README.md) — install + orientation.
