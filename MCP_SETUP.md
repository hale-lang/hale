# Wiring iris into your agent shell

iris ships an MCP (Model Context Protocol) server. When your
agent shell — Claude Code, Cursor, Windsurf, Zed, anything that
speaks MCP — knows where to find iris, it can drive iris on
your behalf: open the window, run interactive demos, read pane
state, narrate what's happening alongside you in your normal
chat surface.

This is iris's primary distribution model. iris isn't a
standalone tool you context-switch to; it's a substrate your
existing agent can pick up and use.

## The flow

1. iris is installed (binary on `$PATH` or known location).
2. Your agent's MCP config points at iris's server-launch
   command (typically `iris --mcp`).
3. In your agent's chat: "show me iris" / "run the permission
   gate demo" / "open foo.hl in iris".
4. The agent discovers iris's tools (`tools/list` over MCP),
   sees `iris.demos.list`, `iris.demos.run`, `pane.read`,
   `viz.scene_info`, etc., and calls them.
5. iris opens (or is already open), drives the relevant state
   changes, returns structured results. The agent narrates
   each step in your normal chat — paced for reading.

iris itself just opens a window and renders the substrate.
The narration happens in your agent's terminal.

## Configuration

### Claude Code

Add iris to `~/.config/claude-code/mcp.json` (or whatever path
your Claude Code install uses; `claude mcp --help` to confirm):

```json
{
  "mcpServers": {
    "iris": {
      "command": "/absolute/path/to/iris",
      "args": ["--mcp"],
      "cwd": "/some/workspace/path"
    }
  }
}
```

After saving, restart Claude Code. `claude mcp list` should
show iris connected.

### Cursor

`File → Preferences → MCP Servers`, add:

- Name: `iris`
- Command: `/absolute/path/to/iris`
- Args: `--mcp`

### Other MCP clients

Same pattern: command = the iris binary, args = `["--mcp"]`.

## Available tools

The agent will see these via `tools/list`. They're the same
ones documented in `AGENT_PROTOCOL.md`.

### Read tools (free; safe to call repeatedly)

| Tool | What it returns |
|---|---|
| `pane.read` | `{path, line, ...}` for the named pane |
| `pane.focus_state` | which pane has focus + visible-panes list |
| `permission.state` | the full PanePolicy matrix |
| `viz.scene_info` | active VizScene's name, domain, channels, state |

### Demo tools (interactive tours)

| Tool | What it does |
|---|---|
| `iris.demos.list` | returns `[{name, summary}]` catalog |
| `iris.demos.run({name})` | executes one demo, returns `{ran, ok, steps}` |

Available demos today:

| Demo | What it shows |
|---|---|
| `hello` | Basic — four panes, focus keys, quit |
| `permission_gate` | Open a file, lock the source pane, watch a write get refused at the bus |
| `viz_modes` | Cycle viz through bulk → harmonic → resolution |
| `scene_info` | Inspect the active VizScene's surface |
| `concurrent_work` | Drive multiple file opens to show the two-channel bus in motion |

### Lifecycle

| Tool | What it does |
|---|---|
| `iris.launch` | summon iris if it isn't already running (Phase 5+ — currently no-op placeholder if iris is up) |

## Narration shape

Demo results are returned as JSON with a `steps` array — one
human-readable string per step. The agent reads the array and
narrates each step to you with appropriate pacing.

Sample call + response:

```
agent → iris : {"method": "iris.demos.run", "params": {"name": "permission_gate"}}
iris → agent : {
  "ran": "permission_gate",
  "ok": true,
  "steps": [
    "Submitting proposal: open foo.hl. Gate allows by default.",
    "Source pane now shows foo.hl. Now locking the source pane against agent writes.",
    "Submitting proposal: open bar.hl on the same channel.",
    "Source pane still shows foo.hl. The proposal was refused at the dispatcher — no mutation reached the pane. Structural enforcement at the bus layer."
  ]
}
```

The agent in your chat reads each step, narrates it (probably
with a 1-2 second pause between steps so you can watch iris's
window respond in real time), and offers to run the next demo.

## Suggested first prompts

Once iris is wired in, try these in your agent chat:

- *"What can iris demo?"* — agent calls `iris.demos.list`,
  presents the catalog
- *"Run the permission gate demo"* — agent calls
  `iris.demos.run` with name="permission_gate"
- *"Show me the four-pane layout"* — agent calls
  `iris.demos.run` with name="hello"
- *"What's in iris's source pane right now?"* — agent calls
  `pane.read` with pane_id="source"
- *"What scene is the viz pane on?"* — agent calls
  `viz.scene_info`

## How this maps to VISION

This is the "summoned, not switched-to" model from VISION §5
in concrete form. You stay in your agent shell. iris becomes
the substrate your agent reaches for when you need to think
about structural information visually — locus topology, model
state, search graphs, anything iris's domain plugins ship a
VizScene for.

The demos are also rehearsal: they teach you (and your agent)
what iris is for by showing the wedge moments — bus delivery,
permission enforcement, scene introspection — in 4-5 step
chunks. Once you've seen them, the same primitives are
available for real work.
