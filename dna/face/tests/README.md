# The face: browser integration

These Playwright tests run Chromium against the real Hale API and temporary Git
Records. A native Hale writer imports the HTTP suite's fixtures, producing real
canonical practice receipts and journal events. No database, organization body,
model, external identity provider, or production Record is used.

From the repository root, with compatible compiler and API binaries already
available from a Hale build or upstream artifact:

```sh
export HALE_BIN="/absolute/path/to/hale"
export HALE_API_BIN="/absolute/path/to/hale-api"
cd dna/face
npm ci
npx playwright install chromium --only-shell
npm test
```

The runner compiles only the Record writer in owned temporary storage; it does
not rebuild the API, composed catalog head, private Knowledge service or scripted
command adapter.
Compiler and API binary paths can point elsewhere. Git configuration and
plumbing overrides, database URLs and model credentials are isolated. Each test
owns its Record, dynamic listener and API child; failure teardown stops the child
and removes scratch storage. Tests have no automatic retries.
On Linux, `prlimit` bounds compiler processes to 2 GiB address space and native
fixture processes to 512 MiB, with 30 CPU seconds and core dumps disabled. These
limits are per process; the serial browser worker keeps concurrent fixtures
bounded. Browser/V8 processes use their own normal address-space configuration.

Practice/review navigation, opaque IDs, Unicode and multiline text, browser
history, missing objects, empty catalogs, snapshot conflicts and receipt
redaction use actual API responses. Authentication loss and source failure use
intercepted 401/503 responses to exercise browser state transitions; real OIDC
callbacks and authenticated reads remain covered by `dna/api/tests`. Delaying a
real detail response tests that obsolete data cannot overwrite another workspace.

Organization tests commit the native seed in `tests/organization` and a separate
domain ownership map into their temporary Git Record. The API invokes the actual
compiler against that committed source; tests do not substitute a hand-authored
topology artifact. Repeated nested instances, the explicit positions group,
structure outside that group and an uninstantiated declaration remain distinct.
Dirty source, changed committed source and invalid committed source exercise the
real provenance, conflict and failure paths. These are declaration inspection
tests, not proof of semantic position administration or live occupants.
One fixture uses ordinary `hale dna new` with an owned toolchain cache, preserving
the generated, ignored `vendor/dna` import. It never runs the organization or a
model. Valid, invalid and missing ignored dependency bytes exercise cache
invalidation at unchanged source HEAD, recovery and preservation of project refs,
worktree state and dependency contents. A larger native source fixture covers
parent inspection outside the current page and return to the original scope.

### Organization publication UI contracts

`organization-publication.spec.mjs` and `organization-source-controls.spec.mjs`
each contain five opt-in UI contract cases. They serve the current web assets
on a disposable loopback listener and replay retained native read evidence.
They do not compile a fixture, run a native service, publish source, or prove
native command authority or effects. Capabilities, submission receipts and
recovery responses are explicitly scripted; the chart, exact source, changed
draft validation and exact Review candidate reads come from the native API.

From the repository root, after installing the browser test dependencies and
Chromium as above:

```sh
HALE_ORGANIZATION_PUBLICATION_EVIDENCE="/absolute/path/to/draft-api" \
HALE_ORGANIZATION_REVIEW_EVIDENCE="/absolute/path/to/candidate-api" \
  node dna/face/node_modules/@playwright/test/cli.js test \
  --config dna/face/tests/playwright.config.mjs \
  organization-publication.spec.mjs organization-source-controls.spec.mjs
```

The draft directory must contain `capabilities-response.json`,
`organization-response.json`, `draft-response.json`,
`draft-validation-request.json` and `draft-validation-response.json` from one
unchanged application/base. The candidate directory must contain the exact
native `candidate-response.json` and `review-response.json` pair. Missing
environment variables visibly skip their cases; supplied invalid evidence fails.
The draft interceptor only accepts the exact source/base request captured by
native validation. An arbitrary edited source never inherits that validation.

These contracts cover independent source grants, exact proposal/verdict payloads,
source invalidation, proposer independence, recoverable v3/v4 request metadata,
GET-only reload after a lost reply, and separate source/Review/application/running
stages. They retain desktop and narrow-screen comparisons and confirmation/
recovery images. Native publication, Review, apply and observed-host acceptance
remain separate required gates.

Definitions, Knowledge and scripted command adapter integrations are opt-in.
Default runs visibly skip their cases when `HALE_FACE_CATALOG_BIN`,
`HALE_FACE_KNOWLEDGE_BIN` or `HALE_FACE_COMMAND_BIN` is not supplied.
The launcher never compiles these compositions automatically. Each optional value must be an absolute path to a
readable, executable file implementing its existing fixture interface. The runner
validates supplied paths, passes them to Playwright unchanged and leaves those
binaries in place after teardown. A missing file, relative path or inaccessible
supplied binary fails setup rather than silently skipping or substituting
responses. Supplying one provider enables only that integration lane.

Provider source is not part of this browser increment and need not be present
in the checkout. Obtain compatible prebuilt fixture artifacts from their upstream
owner or build them explicitly in the checkout that owns their implementation.
With the compiler and API variables above set, run either optional lane from
the repository root:

```sh
HALE_FACE_CATALOG_BIN="/absolute/path/to/catalog-provider" \
  npm --prefix dna/face test -- definitions.spec.mjs
HALE_FACE_KNOWLEDGE_BIN="/absolute/path/to/knowledge-provider" \
  npm --prefix dna/face test -- knowledge.spec.mjs
```

Supply both provider variables and omit the final spec argument to include both
integrations in the full browser run. This is an API/browser integration gate;
the launcher does not run upstream engine or PostgreSQL suites. Skipped cases
and contract fixtures do not count as live integration success.

## Definition authoring

`definition-drafts.spec.mjs` uses the same real native application catalog as
`definitions.spec.mjs`, with the separately advertised draft-validation profile.
The lane exercises N+1 editing, ordered Step/member controls, exact signed Int64
values, Unicode and literal markup, native missing-reference and revision
collision refusal, source/authentication changes, late results, response digest
binding, immutable Hale downloads, and mobile keyboard navigation. It checks that
validation changes neither the loaded catalog nor project refs. Deliberately
altered responses establish browser refusal behavior only.

```sh
HALE_FACE_CATALOG_BIN="/absolute/path/to/catalog-provider" \
  npm --prefix dna/face test -- definitions.spec.mjs definition-drafts.spec.mjs
```

The native operations proof separately compiles the generated registration fragment
and compares its full catalog encoding. Browser export alone does not establish
that a source fragment compiles. Draft validation/export does not prove original
source roundtrip, governed publication, activation or real DNA command recovery.

## Practice command conformance

`commands.spec.mjs` uses native practice reads with scripted command capability
and receipt responses. It checks exact replacement comparison, literal text,
UTF-8 byte limits, principal preconditions, metadata persistence before POST,
storage failure, two-tab reservation, lost replies and lookup recovery. It also
checks that identity changes clear confidential data, late responses cannot
restore it, write revocation and failed collection reads preserve recovery, and
approval remains distinct from adoption. Default startup remains read-only.

`verdicts.spec.mjs` adds exact-candidate decisions over real native pending Reviews
and practice receipts. It checks the same-snapshot candidate join, deliberate
choice, literal optional notes, distinct Review/subject identities, independent
capabilities, protected/mismatched candidates, legacy recovery, cross-operation
reservations and a refused request beside another command's approved/adopted
subject. Dismissing a completed request refreshes native reads so a newly
redacted candidate cannot retain stale eligibility. Scripts still cannot
establish native authorization or durability.

`commands-native.spec.mjs` sends real HTTP requests through the Hale adapter with
an explicitly scripted `CommandProvider`. Build `dna/api/tests/commands` explicitly
and supply its executable to enable this lane:

```sh
HALE_FACE_COMMAND_BIN="/absolute/path/to/scripted-command-api" \
  npm --prefix dna/face test -- commands-native.spec.mjs
```

The harness starts it as `COMMAND_BIN ROOT PORT WEBROOT` with
`HALE_FACE_SCRIPTED_COMMANDS=1`. It uses native Record reads and the real
authentication, Origin, codec and receipt validation paths for both operations,
including cross-operation request-key conflict. The provider stores
request metadata in memory and reads `ROOT/command-mode` for scripted results;
its candidate and Review references are synthetic. Browser reload recovery is
tested while that process stays alive. These cases prove adapter composition,
not native admission, process-restart durability, Review settlement or adoption.
They assert that Record refs are unchanged. Never use this fixture as an
administration server for real data.

## Optional fixture executable contracts

These are test fixture interfaces, not arbitrary production-server entrypoints.
The harness owns a fresh `ROOT` under `/tmp/hale-face-browser.*`, selects dynamic
loopback ports and starts each service with `ROOT` as its working directory.
The catalog and Knowledge fixtures must produce real native data and API responses
matching the browser specifications; a successful response stub does not satisfy
those integration gates. The separate scripted command lane has the deliberately
narrower conformance scope described above.
The harness terminates its children and removes `ROOT` after each case.

The catalog fixture is invoked as `CATALOG_BIN ROOT PORT WEBROOT`, replacing the
normal public API for Definitions cases. It must serve the supplied static
assets, the normal API startup/authentication contract and the application's
native catalog. The baseline Record writer prepares `ROOT` and `fixture.json`.
Readiness requires `/api/hale/v1/applications` to return that manifest's
`application` value as `source.record_id`. The fixture reads `ROOT/catalog-mode`:
missing or `original` selects the initial catalog; `updated` selects the changed
catalog; `unavailable` and `invalid` exercise their declared failure contracts.
The catalog's exact names, revisions, ordered leaf/child Steps and dependents
are specified by [definitions.spec.mjs](definitions.spec.mjs). Tests cover stale
catalog snapshots, unsupported/unavailable/invalid states, session loss, exact
cross-links, mobile focus, history and literal rendering of source text.

The Knowledge fixture has three command forms:

- `KNOWLEDGE_BIN ROOT seed COUNT` creates canonical Git receipts and events
  and `ROOT/fixture.json`, preparing the service's native relationship fixture.
  The manifest supplies
  `application`, `knowledge`, `predecessor`, `proposed`, `hidden`, `name`, `text`,
  `target` and `neighbors` (objects with an `id`). Exact fixture expectations are
  in [knowledge.spec.mjs](knowledge.spec.mjs).
- `KNOWLEDGE_BIN ROOT PORT` serves private Knowledge reads on loopback.
  `/identity` must return JSON with `identity` equal to the manifest's
  `application`. Restarting this command against the same root and port must
  recover the current native data.
- `KNOWLEDGE_BIN ROOT ACTION` applies native mutations for `advance`, `protect`,
  `protect-predecessor` and `redact`.

The harness sets `HALE_DNA_KNOWLEDGE_DSN=memory` and a disposable fixture-only
`HALE_DNA_KNOWLEDGE_READ_KEY`. It passes the private origin through
`HALE_DNA_KNOWLEDGE_URL` to `HALE_API_BIN`, which must support the corresponding
authenticated private graph-read protocol and public Knowledge contract. The
public API still receives `ROOT PORT WEBROOT`. Thus both supplied binaries must
be mutually compatible; setting the environment variable alone does not
establish an available provider.

Knowledge cases require real canonical receipts, lifecycle events, projected
nodes/bindings and native stored relationships. No successful graph response is
mocked. Tests cover 33-item cursor paging, 32 stored relationships,
target relevance, missing/protected equivalence, removal of a newly protected
predecessor and its relationships, Record snapshot changes, actual service
stop/restart, literal text and mobile inspection. The relationship map is checked
against the actual returned edge page for directions, labels and endpoint
identities. Its navigation retains context, snapshot and list cursor, and its
mobile keyboard order follows the focused item and connected items. Names come
only from the current collection; the map triggers no additional graph reads.
Only the 401 browser-clearing case injects an error. PostgreSQL,
projection-lineage failures and concurrent
Record/Ledger/store mutations remain upstream verification responsibilities.
Earlier local native results are historical evidence for those implementations,
not proof that this browser increment includes them.

The browser cases also include a stalled read that expires after the 15-second
deadline and recovers through the real service. Native short commands share the
Linux `/tmp/face-native-validation.lock`; long-running fixture services retain
their per-process limits without holding that lock.
`PLAYWRIGHT_BROWSERS_PATH` can select an existing Chromium cache. Report acceptance
evidence on [#690](https://github.com/hale-lang/hale/issues/690), identifying the
provider artifacts, executed cases and skipped optional lanes.

Failure traces, screenshots and the API log land in `test-results`. Successful
desktop and narrow practice screenshots are also saved there for visual review.
CI runs the suite on partition 1 after the compiler and API have been built.

## Ordinary Hale application administration

`application.spec.mjs` uses the real native [intake-control application](../../../iris/examples/intake-control/README.md)
and generic application API, in separate processes over a fresh application-owned
SQLite database. No DNA Record, provider or model discovery is started.

```sh
HALE_FACE_APPLICATION_BIN=/absolute/path/to/intake-control npm run test:application
```

The optional lane is skipped when that explicit binary is absent. Build the
example and its focused provider proof using the example's documented SQLite
prerequisites. The harness keeps per-process memory/CPU limits, unsets inherited
DNA and observation settings, and removes only its owned temporary database.

Native HTTP cases verify actual pause/resume and activity, no-op revisions,
competing writers, authority, request equivalence/conflicts, retained refusals,
API restart, application restart and old-worker fencing. Browser cases verify
explicit confirmation, reservation before transmission, lost reply after a real
commit, GET recovery without resubmission, state-unavailable recovery, receipt
binding, principal changes, storage failure, keyboard control and mobile layout.
Transport faults are injected around the native response; the application's
state and receipt decisions remain real. This lane does not establish DNA
practice admission, Review settlement, adoption or workflow execution.

The optional `HALE_FACE_WORKFLOWS_BIN` fixture implements `seed`, `members`,
`finish`, `cancel`, `redact`, `adopt` and `invalid` against its owned temporary
Record. Source: `dna/api/tests/workflows/main.hl`. It writes sample facts with
native codecs; it is not a live executor or proof of upstream durability/recovery.
`workflows.spec.mjs` checks native recipe/Step/attempt presentation, recursion,
barriers, cancellation, visibility, unavailable sources and mobile deep links.
`workflow-interaction.spec.mjs` checks the adjacent workflow/focus layout,
Step-local barrier navigation, child/containing-Step return paths, exact attempt
selection through reload/history, cross-execution selection clearing, keyboard
and narrow-screen use. Its final case explicitly overlays unknown/ambiguous
pending keys in browser reads; that conformance overlay does not claim native
admission or persistence. Other successful reads use the native recorded fixture.
The launcher skips this lane visibly unless the native fixture is supplied.

## Organization source and startup

`organization-structure.spec.mjs` uses the same real native Organization draft
API as the source editor. It checks child creation and exact export, shared-field
rename, precise Int64 defaults, explicit empty-position-group semantics, removal
impact, source/comment preservation, stale bindings, native refusal and source
races, along with keyboard/narrow-screen use. It never changes the fixture's
project refs, source files or Record. These checks do not establish publication,
activation, ownership reassignment or live obligation retirement.
Move cases cover shared source/destination impact, constructor preservation,
exact validated export, acknowledgement invalidation, duplicate names and
recursive containment through the same native draft API.

`organization-impact.spec.mjs` adds six interaction cases using a real native
move into a shared parent. Captured before/after projections establish every
comparison node, change marker and containment edge. The cases check stable
positions across source planes, exact inspector facts and candidate-only editing,
branch entry and return, preservation of source/project state without extra API
traffic, and invalidation of the chart and export after a source edit. A 390px
reduced-motion case exercises plane selection and instance inspection by keyboard,
checks page overflow and captures the resulting view. These are checked source
previews, not evidence of publication or live reassignment.

`organization-navigation.spec.mjs` checks branch entry against the native source
projection: exact root and immediate children, native-parent breadcrumbs, selected
instance independence, positions/all filtering, deep links, reload/history and
narrow-screen keyboard use. Its paged fixture reads an off-page root at the exact
collection snapshot without claiming complete child coverage. Authentication loss
clears the branch; mismatched root identity or source basis clears the whole
unverifiable view. Missing IDs remain literal text with an explicit route out.
These seven cases exercise source navigation, not effective authority or live work.

`ownership-drafts.spec.mjs` uses the native optional ownership draft endpoint.
It covers exact CRLF/comment preservation, assignments and inheritance,
single-owner mode, membership and host review impact, Unicode, narrow-screen
use, source refusal, source races, access loss, framing/context boundaries and
the disabled host profile. It checks project refs/status stay unchanged.

`startup.spec.mjs` exercises the checkout launcher with a real disposable
`hale dna new` project, its generated source committed only in the test worktree.
It covers building and serving the native API, captured vendored Organization
source, existing-binary startup, independent process shutdown, temporary-build
cleanup and incomplete private-service configuration. It does not seed a sample
Record/catalog, run an organization body or model, or initialize Postgres/Compose.
Native processes remain bounded; these are startup checks, not foundation suites.

## Knowledge interaction

`knowledge-interaction.spec.mjs` covers the connected Knowledge map/editor:
exact-edge selection and removal, named endpoint selection, incoming candidate
direction, current/draft comparison without requests, draft preservation,
source-check invalidation, off-page endpoint lookup, access loss, asynchronous
focus continuity and narrow keyboard use. Eight cases reuse the native Knowledge
fixture. One explicitly labeled browser-only overlay adds parallel/self edges
for selection conformance; it does not assert native persistence of those edges.
The exported removal in that case still targets the original native edge.

Draft graph marks are local presentation. No test here establishes graph
publication, service admission, authority or durable command outcomes.

## Native Knowledge relationship acceptance

`native-knowledge-browser.spec.mjs` drives the existing relationship map and
editor against the public API and real native Knowledge command service. Supply
three prebuilt, absolute binary paths and run from `dna/face`:

```sh
HALE_KNOWLEDGE_API_BIN=/absolute/path/to/knowledge-api \
HALE_KNOWLEDGE_SERVICE_BIN=/absolute/path/to/knowledge-service \
HALE_KNOWLEDGE_SEED_BIN=/absolute/path/to/knowledge-seed \
node node_modules/@playwright/test/cli.js test \
  --config tests/playwright.config.mjs native-knowledge-browser.spec.mjs
```

The respective sources are `dna/api`, `dna/knowledge/service`, and
`dna/api/tests/knowledge`. These tests never build native code or install a
browser. Without all three explicit paths they skip visibly. The seed binary
creates only prior canonical subjects; it never authors a command outcome.
`native-knowledge-harness.mjs` owns a fresh Git Record, an explicit application
and principal policy, separate private read/write keys, and bounded local service
process groups. Cleanup stops every owned process, including restarted services.

Eleven cases cover a directed Unicode relationship recorded once and observed
through a fresh graph read; an actual admitted POST whose reply is discarded,
followed by API/service restart and GET-only browser reload recovery; stale
Record-head refusal; unavailable graph readback and subsequent visibility
restriction; and Review-required authority with no direct fallback. The browser
saves only scoped recovery metadata before POST. Successful command responses
and graph reads are native. The loss/outage cases inject only transport failures.
Restart opens an empty in-memory graph and reconstructs it from the same Record.

The removal cases select one exact stored relationship and preserve its reverse,
other labels and endpoint items. They cover a lost removal reply followed by
restart/GET-only recovery, stale-head refusal, independently denied/default-denied
removal and unlink-only authority, and native pagination across 27 relationships.
A failed second-page transport leaves absence unestablished; a later GET checks
the complete sequence at one snapshot. An endpoint restriction between admission
and graph read also leaves absence unestablished and clears displayed details.
Saved v2 metadata identifies the original link/unlink operation; the lost-link
case also checks compatibility with existing v1 metadata that implied link.

The receipt establishes the committed Record effect separately from graph
observation. The graph generation is read provenance, not an atomic write
precondition. Removal observation checks every visible relationship page, up to
32 pages within a 15-second continuation deadline, followed by receipt visibility
under the same Record head. A hidden endpoint, changed source, failed read or
unfinished sequence cannot establish absence. Only `edge.link` and `edge.unlink`
submit in this profile; the other five editor operations remain drafts. These
checks establish the fixed-policy, local,
Record-only deployment, not Review-mediated Knowledge changes, Ledger routing,
or multiple clones. Desktop/mobile screenshots, process identities and native
logs are retained under the Playwright output directory.

### Generic Knowledge nodes through native Review

Practice administration reuses the same native node and binding profiles.
`native-practice-lifecycle-http.mjs` covers a compact creation, additional
applicability, revision and retirement sequence with independent Reviews, exact
provenance and retained history. `native-practice-lifecycle-browser.spec.mjs`
starts from the Practices UI and exercises those entry points through real
native services. Both use the five matching binary environment variables below;
run the HTTP script with Node or pass the browser spec to Playwright. These small
flows do not establish sustained or multi-page Knowledge-service stability.

`practice-context-editor.spec.mjs` separately checks the contextual editor's
action families, exact source text, canonical scope, ordinary Knowledge behavior
and rejected mismatched contexts with scripted module callbacks. It requires no
native binaries and does not establish admission or adoption.
`practice-context-routes.spec.mjs` uses explicitly scripted full-app reads to
check exact Practice/graph snapshot joins, inactive-version controls and
relationship selection with return navigation. It also uses no native services;
successful command execution belongs to the separate native browser proof.

`native-knowledge-node-browser.spec.mjs` exercises ordinary Knowledge creation,
revision and retirement through their actual proposals, canonical Reviews and
native activation. It composes `native-knowledge-node-harness.mjs` with the
existing Body/relay/membrane owner. Supply matching prebuilt binaries and run
from `dna/face`:

```sh
HALE_NATIVE_COMMAND_API=/absolute/path/to/composed-review-api \
HALE_NATIVE_COMMAND_BODY=/absolute/path/to/body \
HALE_NATIVE_COMMAND_RELAY=/absolute/path/to/relay \
HALE_NATIVE_COMMAND_MEMBRANE=/absolute/path/to/membrane \
HALE_KNOWLEDGE_SERVICE_BIN=/absolute/path/to/knowledge-service \
node node_modules/@playwright/test/cli.js test \
  --config tests/playwright.config.mjs native-knowledge-node-browser.spec.mjs
```

The composed API is built from `dna/api/practice_review`; all five binaries must
come from the same implementation. The fixture creates an explicit node policy
with separate propose/revise/retire Review grants and exact author/target scope.
The private Knowledge service uses the same Record as the actual Body, with
separate private read/write keys. Every dependent native process joins the
existing bounded owner and is stopped or restarted with it. No seed binary,
authored command outcome, or successful response overlay is used.

The browser creates a non-Practice `idea`, inspects its canonical candidate in
the existing Review screen, approves it, follows revision and retirement, and
checks retained historical receipts. Separate cases cover lost POST response
and complete service restart with GET-only recovery; stale source refusal;
denied node authority despite graph/relationship access; and competing approved
revisions with a distinct activation refusal. Screenshots retain the stages and
literal text on desktop and mobile. Node recovery metadata is version 3 and
contains the original operation and collection/node target identity, never the
draft text. Edge v1/v2 recovery stays in the same request namespace.

Proposal creation is not approval, and Review approval is not activation. The
UI uses canonical receipt reads for decisions, then a fresh exact graph read to
observe adoption or retirement. The final receipt must still be visible at that
graph's exact Record head; a real competing admission between those reads is
covered and withholds graph observation without reversing adoption. Opening
prior history preserves that selection.
These are local, fixed-policy, Record-only acceptance cases; they do not establish
Ledger routing or distributed multi-clone operation.

### Native Knowledge locus bindings

`native-knowledge-binding-browser.spec.mjs` uses the same five explicit native
binaries as the node gate above. Run it with that environment and substitute
its filename in the Playwright command. Its small harness wraps the existing
same-Record composition with independent bind/unbind Review grants and exact
author/target scope pairs. The initial relevance case creates an actual idea
bound to a different branch, so adding support applicability has a real effect.

The browser uses its existing in-place applicability preview, creates a typed
binding-change candidate, inspects the exact canonical document through
`/dna/reviews/candidate`, and decides through the existing Review controls.
Proposal, Review, binding effect and graph observation remain separate. Exact
removal retains a distinct descendant binding and the original Knowledge item.
Settled Reviews retain their canonical binding document with decisions disabled.
The proposer Alice cannot decide her own binding Review. A separately granted
Bob authenticates for the decision; returning to Alice recovers only her original
Knowledge request. Actor switching changes API identity, not a browser authority
claim, and the native service independently enforces Review independence.
Other cases cover lost POST/restart GET-only recovery, rejection, stale admission,
competing approval with refused effect, independent permissions, and complete
binding pagination with an unavailable continuation. Pagination setup uses real
reviewed native commands, not authored binding-outcome rows. That larger setup
restarts the full service composition after each six completed bindings and
once before reading the complete history and removing a binding. Its intended
acceptance is restart projection and paginated observation, not sustained
service operation. This larger real binding case remains blocked: preserved
runs exposed a private Knowledge service SIGSEGV after 22 completed bindings
at a 210-row Record, both without and with those setup restarts. The smaller
successful cases do not establish the blocked pagination acceptance.

Binding recovery metadata is version 4, retaining operation, request/source,
subject and the original author/target identifiers; it never stores rationale.
Those identifiers describe intent and grant no authority. Presence requires the
native binding identity and exact tuple. Absence requires a complete unfiltered
binding sequence for the native subject, bounded to 32 pages and the existing
read timeout, plus a still-visible receipt at that same Record head. A failed,
truncated, hidden or changed read never proves removal. The operator's filtered
workspace stays in context during this independent observation. Existing node
v3 and edge v1/v2 recovery and binding draft exports remain compatible; unbind
submission deliberately omits exported presentation fields such as class and
applicability.

## Native reviewed relationship browser acceptance

`native-knowledge-edge-review-browser.spec.mjs` uses the same five explicit
native binaries as the node/binding gates, through
`native-knowledge-edge-review-harness.mjs`. It keeps the existing graph editor,
directed selection and preview. Its four small real cases cover independent
Alice/Bob Review through link and exact unlink, reverse-tuple preservation,
settled canonical candidate history, lost POST with full restart and legacy v1
recovery after a policy-mode change, rejection, and approved-but-refused tuple
competition. Successful command, Review and effect responses are native.
Desktop and narrow screenshots accompany the actual graph observations.

Direct and reviewed requests share the unchanged edge operation and request
shape. Only the durable returned `relationship` receipt object selects the
reviewed outcome path; current capability mode and saved v1/v2 metadata cannot
reinterpret a historical direct receipt. Proposal, Review, relationship effect
and graph observation remain distinct. Reviewed observation reads the exact
canonical tuple and unfiltered relationship pages at one fresh source, then
rechecks authorized receipt visibility and effect at that same Record head.
No failed, hidden, truncated or changed sequence proves absence. These small
cases do not resolve the separately documented binding large-history crash.

The final case in `native-knowledge-browser.spec.mjs` intentionally changes from
unsupported Review authority to supported reviewed admission. Its standalone
composition has no Body/relay: it verifies a pending durable request and an
unchanged graph, without a direct fallback. It does not establish candidate
creation or end-to-end Review/effect readiness; the new composed gate does.

## Practice interaction

`practice-interaction.spec.mjs` checks the contextual comparison and connected
receipt view over native Practice/Review reads with explicitly scripted command
responses. It covers separated literal/Unicode changes, exact-text toggling,
captured CRLF, returning to an unsent draft, independently selected outcomes,
approval/adoption refusal, a refused verdict beside another decision, recovery
placement and read-outage fallback, uncertainty, and narrow reduced-motion
keyboard use. Selected text-format cases overlay read content to exercise
formatting; those overlays do not claim canonical digest correspondence.

Run this lane with `commands.spec.mjs` and `verdicts.spec.mjs` through the normal
runner. It requires the native API and Record writer, not the optional scripted
native command adapter. These checks establish browser behavior; they do not
establish real domain writes, service authority or restart durability.

## Native Practice and Review acceptance

`native-commands.mjs` is an opt-in standalone Node harness for the real native
command API, DNA body, host relay and membrane client. Supply prebuilt binaries;
the harness never builds native code or joins the default browser lane:

```sh
HALE_NATIVE_COMMAND_API=/absolute/path/to/command-api \
HALE_NATIVE_COMMAND_BODY=/absolute/path/to/body \
HALE_NATIVE_COMMAND_RELAY=/absolute/path/to/relay \
HALE_NATIVE_COMMAND_MEMBRANE=/absolute/path/to/membrane \
HALE_NATIVE_COMMAND_EVIDENCE=/absolute/path/to/evidence \
node dna/face/tests/native-commands.mjs
```

The body and relay sources are `dna/api/practice_review/tests/body/main.hl` and
`dna/api/practice_review/tests/relay/main.hl`. The API source is
`dna/api/practice_review/main.hl`; the relay uses the production membrane helper.
Missing binaries fail visibly. The evidence parent is optional and defaults to
the system temporary directory; each run creates its own Git project and an
explicit application-bound policy granting local `alice` and `bob` board access.

Nine cases exercise replacement, real candidate creation and exact Review
approval/adoption; identical and conflicting retries; GET-only recovery after a
discarded POST reply and API/body restart; two admitted competing verdicts;
approved-candidate adoption refusal; stale subjects; principal isolation; and
authority denial. Literal candidate text, rationale and verdict comments include
CRLF, Unicode and a non-NUL control byte. The harness checks canonical receipt
blob hashes and native Record facts without authoring outcomes or mocking reads.
The final case first checks that all-control maximum fields exceed the independent
32768-byte encoded HTTP body cap and are rejected with 413 without admission.
It then carries an 8192-byte text and 2048-byte rationale mixing control and ASCII
characters through native candidate creation, followed by a 2048-byte control
comment and approval. These valid encodings fit the HTTP cap and check exact
receipt content and adoption under the same process limits. Decoded field limits
do not override the separate encoded whole-request limit.

Native services use the existing 512 MiB/30 CPU-second process bounds without
holding the compiler lock during HTTP activity. Their environments exclude
observation, bus, model and service settings inherited from the caller. Every owned
process group is stopped in cleanup. The retained run directory contains the Git
project, journal, canonical candidate blobs, native logs, request transcript,
receipts, binary hashes and `result.json`; a failed case exits nonzero. This is
local Record/body/host acceptance, not routing-1 Ledger or multi-clone evidence.

### Browser operating flow against the native provider

`native-command-browser.spec.mjs` drives the existing face against that real
service stack. It uses `native-command-harness.mjs`, independently of the
scripted command fixtures. Supply the same four absolute binary paths above and
run from `dna/face`:

```sh
node node_modules/@playwright/test/cli.js test \
  --config tests/playwright.config.mjs native-command-browser.spec.mjs
```

Without all four explicit binaries these cases skip visibly. The three cases
exercise the exact candidate comparison and Review decision through adoption;
an actual native POST whose reply is discarded, followed by fresh API/body/relay
processes and GET-only browser reload recovery on the same origin; and two
replacement candidates whose approved Reviews lead to adoption and adoption
refusal respectively. They assert native Record facts, canonical candidate bytes,
one submission per request, and the browser's separate outcome stages. Successful
command responses and domain outcomes are never mocked. Desktop and narrow-screen
screenshots accompany retained process, binary and Record evidence.

The reusable `startService(options)` fixture owns bounded native process groups
and exposes `stop()` for cleanup. A local preview can supply `prepareProject`
before Body startup, a composed API binary, and explicit startup environment for
Organization validation and private Knowledge reads. Browser cases omit those
sample-preparation hooks: their initial Practice and every tested outcome are
produced by the real Body. The Record-only, fixed-authority deployment limits
remain the same as the native service acceptance above.

### Organization status

`organization-status.spec.mjs` reads retained, unmodified native status and
matching Review responses. Set `HALE_ORGANIZATION_STATUS_EVIDENCE` to a directory
containing `status-response.json` and `reviews-response.json` from the same
captured Source, then run it with the standard Playwright configuration. The
baseline deliberately has historical healthy observation with no current
process association, and source comparison may be unavailable. Discovery,
transport failures, current-running and rollback variants are explicitly
scripted browser contracts; this is not live publication or Host acceptance.
The cases cover independent Review-based reads, reload without command recovery
identity, authority/outage clearing, exact snapshot/candidate/artifact joins,
unknown launch evidence and rollback separation.

This browser suite builds no native services. Live API/Host composition needs
its separate native gate.

### Organization responsibility reads

`organization-impact-read.spec.mjs` uses unmodified native `impact-response.json`
and `reviews-response.json` exports from one captured Source. Set
`HALE_ORGANIZATION_IMPACT_EVIDENCE` to their directory and run the spec with the
standard Playwright configuration. It checks the exact Review/Record join,
independence from source-comparison availability, count clearing after coverage
or access loss, and bounded refresh after a capture conflict. Discovery, count
variants and transport faults are explicitly scripted; the browser sends no
mutation and does not prove an admission barrier or source application.

The standalone responsibility renderer's own contract tests separately exercise
closed-wire validation, overlapping count categories, exact decimal display and
keyboard/narrow layout. Native reader and API evidence establish the supported
recorded lifecycle semantics; neither browser contract substitutes for that proof.

### Handed Task administration

`task-administration.spec.mjs` exercises the standalone renderer with explicitly
scripted Task DTOs: closed history validation, retained lifecycle states,
literal acceptance/evidence content, keyboard and narrow layouts, eligible
recipient selection and duplicate preparation prevention. The module sends no
request and stores no recovery state. `task-administration-read.spec.mjs`
exercises the full app using explicitly scripted read, capability and command
envelopes: exact confirmation, identity storage before POST, separate fresh Task
observation, lost-response GET-only reload recovery, authority/privacy clearing,
malformed receipts and the shared unresolved-command slot.

Run these two browser contracts from `dna/face`, with an available Playwright
Chromium browser; no native binary or build is needed:

```sh
node node_modules/@playwright/test/cli.js test \
  --config tests/playwright.config.mjs \
  task-administration.spec.mjs task-administration-read.spec.mjs
```

These contracts passed six cases each during this increment. They prove browser
behavior, not native authority or durable reassignment. The separate actual
native browser gate below also passed all three cases; an extended first case
then passed real pagination, stale/malformed/absent reads and read-no-write checks.
All473 contract fixtures and five actual response schemas pass. The actual
handoffs explicitly bind no practice in force; nonempty bound acceptance is
covered separately by the focused native Task fixtures.

`native-task-browser.spec.mjs` uses `native-task-harness.mjs` and two explicit,
matching source-built binaries. It creates a fresh Git Record, invokes native
`Dna.ask` handoffs, installs the closed application-bound Task policy and starts
the real composed API. There is no Body, Host or graph service, and successful
command responses and assignment outcomes are not mocked. Its three cases cover
preserved responsibility and exact assignment history; a genuine POST whose
response is discarded followed by API restart and GET-only recovery; and stale
assignment, retired-person and outside-policy refusals.

```sh
HALE_NATIVE_TASK_API=/absolute/path/command-api \
HALE_NATIVE_TASK_SEED=/absolute/path/task-seed \
HALE_NATIVE_TASK_EVIDENCE=/absolute/path/task-evidence \
  node node_modules/@playwright/test/cli.js test \
  --config tests/playwright.config.mjs native-task-browser.spec.mjs
```

Without both binary paths, native cases skip visibly. The harness isolates the
environment, applies the existing native process bounds and stops every owned
process group. Retained evidence includes native Record facts, requests, binary
hashes and process exits. This gate does not cover person retirement commands,
cross-owner transfer, hosted or Ledger administration, or a source-ownership join.

### Projects workspace

`projects.spec.mjs` is the binary-free UI contract for `IrisProjects`. Its
fixture server scripts the four head paths (`/api/hale/v1/head`, `/head/projects`,
`/head/commands`, `/head/logs`): it covers the closed envelope validation (an
extra key at any level, a Record envelope, a mismatched active project and an
unknown receipt state all fail), the detached and attached form sets, browser
side grammar checks that send nothing, the secret forms' NAME and SOURCE
selects with no value field, the spend checkbox gating the models probe, the
receipt lifecycle for `succeeded`, `refused`, `failed` and `outcome_unknown`
(the last names the command, links the run log and offers no retry), the
`data-observation` attribute set only after a fresh head read, the closed
argument sets of the preview, forge and secret forms, the recovery slot
restored as a GET-only lookup, a lost POST response followed by lookups until
it settles, and a busy head. `projects-read.spec.mjs` runs the real plain
`dna/api/api` through `harness.mjs`: the shell's single head probe answers 404,
the page continues to Practices with the Projects entry hidden, the workspace
opened by hand says no project service answers, no
request other than GET is sent and the Record's refs are unchanged.

```sh
node node_modules/@playwright/test/cli.js test \
  --config tests/playwright.config.mjs projects.spec.mjs
HALE_BIN=/absolute/path/to/hale HALE_API_BIN=/absolute/path/to/hale-api \
  npm test -- projects.spec.mjs projects-read.spec.mjs
```

Neither proves a verb ran on any machine. The real-receipt gate against the
built project service is a separate native lane that skips without
`HALE_NATIVE_HEAD_BIN` and `HALE_NATIVE_HEAD_API`.
### Raising work

`task-create.spec.mjs` is the standalone contract for `web/task-create.js`, the
"New task" form (`window.IrisTaskCreate`): exact validation of an outcome for a
locus, the whole organization first and declared working-context loci after it,
byte bounds, literal markup, denied sessions, DOM-tampered choices and the
pending/corrected preparation flow. The module sends no request and stores
nothing. `task-create-read.spec.mjs` exercises the full app with scripted
capability, Task and command envelopes: identity storage before the one POST,
the exact `dna.task.create` envelope with the captured `record_head`, the
organism's answer followed through lookup (`requested` then `born` with its Task
id), a refusal as a completed request, lost-response GET-only reload recovery,
inconsistent capabilities, malformed receipts and the shared unresolved slot.
Both run binary-free from `dna/face`:

```sh
node node_modules/@playwright/test/cli.js test \
  --config tests/playwright.config.mjs task-create.spec.mjs task-create-read.spec.mjs
```

`native-task-create-browser.spec.mjs` reuses `native-task-harness.mjs` and the
same two binaries and environment as the native Task lane. It raises a task from
the real face through the real composed API into a fresh Git Record and
proves one POST, a real receipt, and exactly one `intent.requested` row whose
entity is the receipt's intent id, whose author is the principal, and whose body
begins with the bytes `hale dna ask` writes for the same outcome/from/to before
the command fields; then GET-only recovery across an API restart and a
stale-head refusal. No relay or organism runs in this lane, so the ask stays
`requested`; offer, refusal and birth are the organism's later facts and a
follow-up case. Without both binary paths the lane skips visibly.

### Declared members and recorded assignments

`organization-ownership-people.spec.mjs` passed four standalone browser contracts
for `IrisOwnershipPeople`: exact copied `{owner,name}` selections, duplicate
people in separate declared owner groups, literal Unicode/markup handling,
invalid or unavailable source membership, disabled inspection when Task reads
are unavailable, and native keyboard/narrow-screen behavior. Desktop/mobile
screenshots were inspected. The helper sends no reads, writes or storage calls;
these scripted membership fixtures do not establish native assignment or authority.

`organization-people-routes.spec.mjs` provides separate, explicitly scripted
full-app contracts for declared member → exact `assignee` query, pagination and
reload, filter-response consistency, clearing context on application change,
and an exact selected Task remaining inspectable after reassignment removes it
from the original list. Lost-response recovery remains GET-only when that list
is empty. Owner names are source context, not a Task-owner, position-occupant or
command-authority mapping. Counts are limited to visible native Tasks in the
requested scope.

Run either or both from `dna/face` with an available Chromium browser:

```sh
node node_modules/@playwright/test/cli.js test \
  --config tests/playwright.config.mjs \
  organization-ownership-people.spec.mjs organization-people-routes.spec.mjs
```

All eight full-app route contracts pass. The separate native assignee-filter
proof passes35 HTTP assertions, parser checks,481 contract fixtures and three
actual response schemas. One actual filtered browser case passes native
reassignment, retained selected detail and original-assignee/All-assignees
navigation. The native HTTP terminal case uses a labelled read-projection input;
it does not prove a completion command. No native build is required by the
scripted browser contracts above.
