# Habitat Face prototype

An interactive design seed for the Habitat UI and the default organism view
served by an application's Face. Habitat is itself a DNA application: its Face
adds infrastructure concerns to the organism view, while each application owns
its own emergent Face. This prototype explores that shared visual vocabulary
before integrating authorized Hale-generated `@rpc` surfaces.

The standalone page uses authored fixtures throughout. Account, login, roles,
accessible organisms, administration and command previews have no real authority
or authentication effect. It sends no backend requests or commands and has no
external resources or Codex dependency. Browser-local state only preserves the
preview's selections and settings.

## Run

From the repository root:

```sh
python3 -m http.server 5173 --bind 127.0.0.1 --directory dna/face/prototypes/habitat
```

Open <http://127.0.0.1:5173/>. Alternatively, open `index.html` directly in a
browser. No package installation or Hale compiler build is needed.

## Explore

The sample Atlas organism has 16 body-part pages, each with two graph
perspectives: Head, Face, Heart, Spine, Nerves, Memory, Record, Genome, Body,
Legs, Hands, Hat, Voice, Senses, Reflexes and Skin. Account and organism cards
establish the navigation flow; five theme choices include the system setting.

Each perspective presents stable entities and typed relationships as a 2D route
map or an unfolding 3D graph. Structure, authority, work, signal and evidence
routes have distinct colors. Observed, declared and proposed relationships use
separate line patterns. Depth and proximity carry no authority, ownership or
physical-placement meaning. Graph and outline modes share selection, filtering,
inspectors and a bounded connected-context expansion.

The 32 perspectives include 96 simulated events. A finite scenario starts on
entry; entity and relationship selections replay matching activity, and filters
can restart activity for visible targets. Pause, stepping and log inspection
hold the page for inspection. Traveling solid packets represent observed
message traffic; hollow diamonds trace an event's authored relationships.
Node-only events emphasize their entities. Reduced-motion preferences suppress
travel. A per-page activity log links back to the event and perspective without
duplicating the log entry. Its timestamps are local preview arrival times.

These scenarios replay over a fixed captured graph. Delivery is not completion,
an event trace does not establish new causality, and a pending review or outcome
remains pending. Supporting records and command previews sit below each graph
for later interaction design.

## Source and checks

`index.html` is the generated, self-contained deliverable. Edit `src/shell.html`
and the modular JavaScript, CSS, JSON and page sources under `src/`, then rebuild:

```sh
python3 dna/face/prototypes/habitat/build.py
```

Check that the committed output matches its sources, and run the event checks:

```sh
python3 dna/face/prototypes/habitat/build.py --check
node dna/face/prototypes/habitat/tests/check-event-replay.cjs
node dna/face/prototypes/habitat/tests/check-activity-autostart.cjs
```

For a browser smoke check:

1. Enter Atlas and switch body parts and perspectives; each should start its
   own finite sample scenario.
2. Watch Nerves message packets and Reflexes event traces traverse edges in
   both 2D and 3D. Pause, select an entity and move the camera; the held scene
   should stay paused.
3. Filter Memory or Senses, inspect an event and revisit an activity-log entry.
   Hidden targets should be reported, and log inspection should add no entry.
4. Check the outline at a narrow viewport, change themes, and enable the
   browser's reduced-motion preference.

## Integration boundaries

[Support map](notes/support-map.json) records source references, documented
behavior, proposed read models and unsupported actions per part.
[Design decisions](notes/design-decisions.json) records graph semantics and
implementation handoff constraints. These are prototype research notes, not
new language or API contracts.

The intended integration uses generated `@rpc` clients. User-scoped organism
discovery, administration roles, detailed authorized projections, available
actions and event transport bindings still need verification against the
generated surface. The prototype does not establish endpoint names or adopt
older handwritten Face REST routes as that contract.

Production work must supply source-owned stable identities, exact ownership
and relationship predicates, authorization, coverage, pagination and snapshot
watermarks. Client filtering is not access control. Existing coarse Head
change notices may require authorized projection refreshes; the fixture event
envelope is a presentation model, not a generated subscription schema.
`projection.refreshed` explicitly marks a derived read. Replace the sample
producer with a source-validated adapter and resynchronization behavior before
presenting any activity as live.

Protected Hat context remains lease-scoped, credentials remain redacted, and
command receipt, admission, execution and settlement must remain distinct.
RPC schemas provide typed integration surfaces; each application still owns
the layout and interaction semantics of its Face.
