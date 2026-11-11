# The face

The face is the DNA organism's people surface: three static files
(`web/index.html`, `web/app.js`, `web/styles.css`) served by the standard face
(`dna/face/standard`) in front of the head (`dna/api/project_service`, which
serves surfaces only). It has no build step, no runtime package dependencies,
no database connection and no domain engine: it reads the head's surface and
sends the record's commands, as the signed-in person, and shows a refusal as
the row answered it. Inspecting a running Hale binary is `hale iris`'s job, not
the face's.

The [design direction](DESIGN.md) describes the intended instrument and its
visual acceptance requirements; the [API contract](CONTRACT.md) is the
browser/service boundary it proposed.

## Three screens

- **Organization** is the org chart as the graph states it, read from
  `Commands::org`: every position under what it sits under, who holds it,
  whether its mandate is ratified, and the positions the record proposes and
  the Board has not decided (dashed, with their Review). A grafted organism is
  a child organization, linked to its own face: the face you open is the
  horizon, and its chart is its own record's. Selecting a position shows its
  mandate, its holders and what it signs; filling it (`HoldFill`) and opening a
  position under it (`PositionOpen`) are the record's commands, each a Board
  Review. A stated position also offers to say what it does, to move under
  another position, and to retire: each is a one-step batch (`amend`,
  `move`, `close`), checked first (`BatchPreview`, the chart lines it adds
  and takes out) and then proposed (`BatchApply`) for the Board.
- **Board** is the handed Tasks (`Commands::tasks`, from the record and its
  Ledger as one) and the Reviews in the lanes a board is read by: `Backlog`
  (a Task handed to nobody), `To do` (handed to someone), `In progress` (in
  a transfer, or waiting on something), `In review` (a Review the Board has
  not decided) and `Done` (a decided Task, a settled Review). A fresh
  organization opens its whole starting chart as pending Reviews, so the
  `In review` lane offers to approve them all in one action, each its own
  verdict in the person's name, after the count is shown and confirmed. A card opens the Review or the Task; a verdict
  (`ReviewVerdict`), taking a Task (`TaskTake`) and reporting it done
  (`TaskDone`) are the record's commands.
- **Knowledge** is every node the record holds, by kind, with search: a node's
  text, author, state, its bindings and edges when memory serves them, and the
  proposals as the record alone holds them when it does not. Proposing,
  revising and retiring a node (`KnowledgeNode*`) go to the record's Knowledge
  policy.

Every call is `POST /api/hale/v1/applications/<id>/commands` with
`{"call": <name>, "payload": {…}}`, the reads (`Index`, `Org`, `Tasks`) and the
commands alike, relayed by the head to the surface's HTTP exposure under the
visitor's session; the describe slice (`{"describe": true}`) says which calls
the signed-in person may make, and the face offers only those. The footer
names the record head and revision every screen was read at.

## Run locally

With an installed toolchain:

```sh
hale dna head /absolute/path/to/dna-project
```

From this checkout, building the head's seeds from the working tree:

```sh
./dna/face/start.sh /absolute/path/to/dna-project
```

Either prints the face's loopback URL and the sign-in link. `--port` selects
another port; `--help` lists the rest. The launcher starts the stub OpenID
provider and signs you in through it; the head's memory DSN
(`HALE_DNA_MEMORY_DSN_HEAD`, as `hale dna memory migrate` prints it) lets
Knowledge read memory's projection; without it the face reads the record's
proposals alone.

The habitat is served the same way: `hale habitat up` starts the face over the
habitat's own head, lands a signed-in visitor on the operator face at `/app/`
and serves this face at `/organization`.

## Development and verification

Edit the three files under `web/`, restart the head to load the changed
assets, then reload the browser. The head serves the three assets and nothing
else from the directory; URLs never become filesystem paths.

Browser tests live in `tests/`: Playwright against the real API over a record
the native fixture writes, one spec per screen. From the repository root, with
a compiler and the API built:

```sh
export HALE_BIN="/absolute/path/to/hale"
export HALE_API_BIN="/absolute/path/to/hale-api"   # hale build dna/api -o …
export HALE_DNA_MEMORY_DSN_OWNER="postgres://…"      # a Postgres the fixture may migrate a record into
cd dna/face
npm ci
npx playwright install chromium --only-shell
npm test
```

See [tests/README.md](tests/README.md) for the fixture and its options.
