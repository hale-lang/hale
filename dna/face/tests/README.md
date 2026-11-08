# The face: browser tests

Playwright runs Chromium against the real API (`dna/api`, built) over a record
a native Hale writer seeds (`tests/record/main.hl`, which imports the HTTP
suite's fixtures and writes real practice receipts and record rows into memory).
No organization body, model or external identity provider runs; the API signs
the browser in with its launch token.

```sh
export HALE_BIN="/absolute/path/to/hale"
export HALE_API_BIN="/absolute/path/to/hale-api"
export HALE_DNA_MEMORY_DSN_OWNER="postgres://…"   # the fixture migrates a record there and drops it after
cd dna/face
npm ci
npx playwright install chromium --only-shell
npm test                        # every spec
npm test -- board.spec.mjs      # one
```

`tests/run.mjs` builds the record writer and the memory fixture into temporary
storage, then runs `playwright.config.mjs` (one serial worker, no retries,
1440×980). Each test owns its record, its API child on a free port and its
scratch directory, and removes them when it ends.

## The fixture

`harness.mjs` extends Playwright's `test` with `service`: the API's origin,
`url(view, params)` (the signed-in shell at `#/<view>?app=…`), the seeded
practice (`name`, `text`, `practice`, `review`, `question`) and a pending one
(`pending_practice`, `pending_review`, `pending_question`), `refs()` (the
record's head and the project's Git refs, to prove a read wrote nothing) and
`mutate(action)` (the writer's actions: `advance`, `redact`, …). Options, set
with `test.use`:

- `seats`: positions the head's own person holds, as `graph.node` and `holds`
  rows the record takes before the API starts (`record-seats.mjs`); the chart
  draws them, and the person's forwarded commands pass their gates.
- `recordCount`: how many practices the writer seeds.
- `knowledge`, `organization`, `definitions`, `workflows`, `commandAdapter`:
  the provider lanes (`HALE_FACE_KNOWLEDGE_BIN`, …), each skipped when its
  binary is not supplied.

The `page` fixture fails a test on any unhandled JavaScript error.
`command-wire.mjs` has `isWrite(request)` for proving a screen sent no
mutation; `http-fixture.mjs` intercepts responses for error paths.

## The specs

- `organization.spec.mjs`: the chart draws the record's seated positions with
  their holders, the inspector shows a position and its mandate state, an
  unknown id selects nothing, and reading writes nothing.
- `board.spec.mjs`: a pending Review needs a verdict and a settled one is
  settled, a card opens its Review with its question, the handed Tasks column
  says why it is empty on a head that cannot read the Ledger.
- `knowledge.spec.mjs`: the record's nodes by kind, a node's literal text with
  injected markup rendered as text, search narrows the list, and reading
  writes nothing.
