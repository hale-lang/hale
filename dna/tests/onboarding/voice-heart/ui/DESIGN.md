# The admin UI: design direction

Voice's admin UI is an instrument for running an inference fleet: seeing
headroom, spend and health at a glance, and changing configuration with
confidence. This document records its visual direction, the rules that keep
it legible, and how it is layered so the parts that are not about voice can
become a library other hale projects reuse.

## One hale design language

Hale already has a visual direction, written for DNA's face:
[`dna/face/DESIGN.md`](https://github.com/hale-lang/hale/blob/main/dna/face/DESIGN.md).
It calls the surface a *high-fidelity laboratory*: a quiet carbon field,
fine vector boundaries, an operator who can tell the selected object, its
context and the evidence for every visible claim at a glance. Voice adopts
that direction rather than inventing a second look, and is the first project
to turn it into packages. The face itself is out of scope here; its document
is the source of the aesthetic, and it is cited, not copied.

From it, the parts voice keeps to:

- **Surfaces** in muted carbon and slate (`#0D0D11`, `#14141A`), restrained
  depth, crisp rules. Off-white for content.
- **Color has meaning.** Cyan and blue for focus and evidenced activity;
  amber for waiting and unresolved; coral for failure. Every state also has
  text or a shape, so color is never the only signal.
- **Type.** A technical sans-serif for reading, monospace for identifiers
  and values, tabular numerals, local font stacks. Long names and complete
  opaque ids fit without displacing controls.
- **Space.** A four-pixel rhythm; tighter within a property group, wider
  between concerns. Square or subtly chamfered edges.
- **Layout.** A context rail (where am I, as whom), a canvas (the primary
  reading surface: here, lists and tables), an inspector (the selected thing
  in full), and an evidence strip (source, revision, freshness). On narrow
  screens these become stages with a clear way back.
- **States are designed.** Loading, stale, unavailable and missing each look
  like something, and a failed read clears data it can no longer vouch for.
- **Operable without pointing.** Keyboard navigation, visible focus, deep
  links that preserve selection and paging, reduced motion honored with a
  fully informative still rendering.

## Color is earned

Hale's runtime observer, iris, was drawn before the face's document and
draws a different thing (a running program's topology), but its rules about
legibility generalize, and voice adopts them. Iris's own statement of the
first one: *structure is slate and quiet; color is earned by activity.*

- **Quiet at rest.** A healthy, idle fleet is slate and off-white. Cyan
  appears where requests are flowing, amber where something waits or a
  quota is under pressure, coral where something failed or is exhausted.
- **Volume is logarithmic.** Request counts span orders of magnitude, so
  bars, sparklines and any encoding of volume step on `log10(1 + n)`: one,
  a hundred and ten thousand requests are three different things and look
  like it.
- **Rare events linger.** A failed request, a refused seat, an exhausted
  account gets an afterglow: drawn in its color and fading over about ten
  seconds, so it is noticed even in a list that keeps scrolling. Sustained
  activity gets the opposite memory, a slow-decaying warmth, so a hot seat
  stays visibly hot through a lull.
- **Aggregate the boring, label the hot.** Twenty seats on a node collapse
  to a summary row with the busiest few shown. Heartbeats and health checks
  never draw. Labels go to what is active; everything is one expansion away.
- **Focus dims the rest.** Selecting or hovering a thing recedes everything
  unrelated, so a dense table still reads.
- **Alerts override.** Coral is reserved for failure and exhaustion, and an
  alert shows regardless of what a view is otherwise coloring by.
- **Ground truth beside intent.** What was configured next to what is
  reported: a seat's assigned config beside the state its node reports; the
  route table's rank beside where requests actually went; a budget beside
  what was spent. The difference is the information.

## Personality

The rules above are the grammar. These are the decisions that make the
look recognizable at a glance, and the ones a contributor is most tempted
to soften. Call it **the readout**: a surface that reads like an
instrument's display, not like a dashboard.

- **Type is the identity.** IBM Plex Sans for reading, IBM Plex Mono for
  identifiers, values and readouts, both shipped with the tokens package
  (Open Font License) and loaded from the same origin, never a font
  service. Two weights each, 400 and 500; no bold. Mono is not an accent,
  it is most of what is on screen, because ids and numbers are the content.
- **Edges are cut, not rounded.** No border radius anywhere. A panel is a
  hairline rule and a chamfer: its top-right and bottom-left corners cut at
  45°, six pixels, the way a machined plate is relieved. Buttons and inputs
  are square. Separation is a rule, not a box, wherever a rule will do.
- **Every panel carries a readout.** Beside its title, in mono, small,
  tracked caps and faint: what it counts and how fresh it is (`2 SEATS ·
  3S`). The evidence strip is the global readout; panels have their own.
  A reading with no readout is decoration.
- **Selection is a hairline, not a fill.** The selected row gets a
  two-pixel cyan rule on its leading edge and nothing else. Hover is one
  step of surface. Focus is a square outline in the focus color.
- **Motion follows evidence.** Afterglow, completed-request traces in the
  Network view, and a slow pulse on the word *fetching* in the evidence
  strip. Nothing else moves: no transitions on
  layout, no hover animation, no skeleton shimmer, no easing on numbers.
  Reduced motion gets stillness, fully informative.
- **Five glyphs, no icons.** `·` quiet, `●` active, `◐` waiting, `✕`
  failed, `✓` confirmed. Always with a word. No icon font, no icon set.
- **The grid is the field.** The canvas shows a faint 32px rule grid, the
  paper the readings sit on. Panels sit on it; they do not float over it,
  so no shadows.
- **What it refuses:** drop shadows, gradients, rounded pills, badges with
  colored fills, toasts, modals (the inspector is the second surface and
  the only one), decorative charts, loading spinners, and any color that
  is not earned.

## The screens, as instruments

Every number the API returns is the content; the UI's job is to make the
right ones comparable.

- **Overview.** Headroom per account (each window as a gauge with its reset
  time), node and seat health as a quiet table that colors only what is
  wrong, spend this period per project against its budget, and recent
  requests with afterglow on failures.
- **Usage.** Summaries by project, key, model, account and day as tables and
  sparklines; the records behind them, filterable by every field the API
  filters by. Tabular numerals, right-aligned, units muted.
- **Accounts.** Windows, limits, the seats that spend it, and whether it is
  exhausted and until when.
- **Nodes.** Capabilities as declared, the effective cap and what bounds it,
  local controls as reported, and the seats with assigned beside reported.
  Enrollment and token rotation, with the token shown once.
- **Seats.** A form per engine, rendered from its `config_schema`; the
  node's verdict (`refused` with its reason) beside the form.
- **Route table.** Per model, the ranked slots and why each is where it is,
  and which seat took the last requests. The one place routing is explained.
- **Catalog, projects and keys.** Budgets and limits with what is spent and
  remaining; a key's secret shown once.
- **Changes.** The audit log, each entry with before and after.

Live state comes from the admin event stream (`GET /admin/v1/events`); the
UI never polls for what the stream carries.

The Network screen shows the fleet as a stable constellation of projects,
models, seats, nodes and accounts. Configuration draws solid links; recent
usage draws dotted project/model links. Completed usage events briefly trace
their recorded path; reported in-flight counts mark occupied entities. Neither
is token streaming or a hardware measurement. Selection isolates a neighborhood
and opens its evidence. Reduced motion keeps a still highlight.

A separate, labeled demo fleet exercises synthetic traffic while the backend
is a stub. It never sends inference or configuration requests. API mode clears
the view on failed reads and does not animate historical SSE replays as live
traffic. The graph is bounded to 160 visible entities, with truncation stated.
This network activity is a second permitted form of motion beside afterglow.

## Layers, so the reusable parts rip out

Three layers; the framework opinion lives only in the top two.

| Layer | Package | Holds | Framework |
|---|---|---|---|
| 0 | `tokens` | Color, type scale, spacing, radii, motion durations, layers: a JSON source of truth compiled to CSS custom properties. | none |
| 1 | `components` | The shell (context rail, canvas, inspector, evidence strip), tables with tabular numerals, gauge, sparkline, status glyphs, afterglow, the schema-driven form, the shown-once secret, the four designed states. | React |
| 2 | `apps/admin` | Voice's screens, the client generated from `openapi.yaml`, its queries and event-stream subscriptions. | React |

`ui/` is a workspace from the first commit, with `packages/tokens`,
`packages/components` and `apps/admin`. Nothing in `packages/` may import
from `apps/`, and nothing in `packages/` knows voice's nouns: a gauge takes a
fraction and a reset time, not an account. Extraction is then moving
`packages/` to a `hale-lang/ui` repository and publishing; `apps/admin`
depends on the published versions and nothing else changes.

Tokens are framework-free on purpose: they are where the look lives, they
survive a change of framework, and a vanilla JS surface can consume them
today. Components are React because it is the stack agents write most
reliably; web components would be more reusable and slower to build. If a
hale project ever needs a graph engine, it is a fourth package,
framework-free like tokens, and iris is where hale's one existing engine
lives.

Dark first. The tokens are written so a light theme is a second set of
values, not a rewrite, and it is not built until something needs it.

## Acceptance

A slice is done when it can be shown at desktop and narrow widths with the
rail–canvas–inspector hierarchy clear, contrast legible, focus visible and
unclipped, dense data and long identifiers unbroken, every state (loading,
stale, unavailable, missing, empty, error) reachable and designed, and every
drawn value traceable to a field the API returned. Screenshots against the
stub are the evidence.

### Unified network observatory

The Network canvas uses lime projects, magenta models, cyan seats, blue hosts,
violet accounts, orange repositories, mint API instances and lavender optional
context. Shape and labels duplicate category color; halos show activity, amber
and coral retain state meaning.

There is one shared field with aligned neural layers leading toward models.
Model labels sit to the right of a compact terminal stack; hosts connect to seats
through orthogonal branches. Only present stages take space; repositories and arbitrary context sit
near projects, and accounts form a vertical layer between seats and models. Directed springs preserve
their horizontal span, while label-aware collision forces separate neighborhoods.
Layer alignment survives viewport changes; selection never reheats the layout. Search and additive selection compose any set of entities, intersecting
complete request membership by default; an explicit Any selected mode unions it.
Resource filters and present-only layers live behind Refine network.

Every usage ID owns its participants, context links and directed execution links.
Execution ends at the actual served model, separately from the requested model
constraint when those differ. Project → API is the ingress hop. The account layer represents budget attribution, not an extra network hop. Repository/metadata correlations
light alongside execution; requested-model constraints never create shortcut edges. Unattributed requests
have no fabricated execution chain. Historical evidence survives configuration
changes. A bounded 160-entity view declares truncation.

Optional metadata dimensions are discovered from returned fields, with labels
and markers that need no registry entry. `metadata.repository` remains an optional
caller-supplied label; other keys are explicitly context, never trusted runtime
identity. UsageRecord still lacks the API instance from `spec/store.md`'s ledger.
Demo-only observation context supplies it until backend work exposes the field.
The demo includes missing repositories, missing API identity and refusals without
seats. It contacts no homelab hosts or providers.

Dragging or arrow keys pin entities; Escape releases a focused pin, Reset layout
clears pins. Reduced motion keeps static evidence. Narrow screens scroll the
shared graph horizontally.

The synthetic demo exposes seven projects, three repository labels and 24 model
aliases. Up to three distinct projects run concurrently, each with a stable path
color and an in-flight selector. Seat and host occupancy derive from that schedule;
completed usage is emitted only when a worker finishes. Demo controls freeze both
the schedule and animation, and the chosen data source survives page reloads.
