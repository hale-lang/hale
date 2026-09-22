# Iris cockpit

A browser cockpit for ordinary Hale application controls and runtime observation,
plus DNA organization, workflow definitions, Knowledge, practices and reviews.
The frontend is eleven static files, served beside the native Hale API or by an
independent static host. It has no
build step, runtime package dependencies, database connection or domain engine.

The [design direction](DESIGN.md) describes the intended spatial instrument
and its visual acceptance requirements.

## Run locally

From this checkout, start Iris against an existing DNA project with one command:

```sh
./iris/cockpit/start.sh /absolute/path/to/dna-project --source-drafts
```

The project argument is optional. With one, the launcher serves that project
through the composed API as before. Without one it runs the project service
(`dna/api/project_service`): the operator-machine head that serves the same
shell, answers `/api/hale/v1/head`, and proxies Record reads to a per-project
API child once a project is attached. The browser then opens on **Projects**,
where a project is created, initialized or attached from the browser; see
[Projects and the project service](#projects-and-the-project-service).

The launcher builds this checkout's native Hale API in temporary storage, serves
the bundled browser, and prints its loopback URL. Set `HALE_BIN` if the compiler
is not on `PATH`. `--port 8793` selects another port. `--source-drafts` opts into
Organization and ownership preparation; omit it for reads alone. Ctrl-C stops
this API and removes its temporary build. The application's existing body and
services retain their lifecycles. The launcher never initializes or commits the
project, adopts its Ledger, starts infrastructure or fetches dependencies.

For a fresh project, create it with `hale dna new /absolute/path/to/project` and
commit the generated source before inspecting Organization. Organization reads
use committed source and existing vendored dependencies; they exclude local
uncommitted edits. Launching Iris itself does not require a running body or a
database for Record/source reads. The generated Compose file supplies Postgres;
it does not currently launch this browser/API or the full DNA service stack.

To use an already-built API, including an application's own catalog and command
provider composition:

```sh
./iris/cockpit/start.sh /absolute/path/to/dna-project --api /absolute/path/to/hale-api
```

`HALE_API_BIN` is the equivalent environment setting. That binary must accept
`PROJECT PORT WEBROOT` and implement the public cockpit API. The standalone API
keeps Definitions and durable commands unavailable until a real application
provider supplies them; the launcher never substitutes a sample catalog.

Existing private Knowledge wiring is inherited through
`HALE_DNA_KNOWLEDGE_URL` and `HALE_DNA_KNOWLEDGE_READ_KEY`. Set the same graph-read
credential on the state service and API; it stays out of the browser and launch
output. Set both values together. Neither service URL nor key establishes that
the service is reachable or up to date; its read responses remain authoritative.
`HALE_IRIS_OBSERVER_ORIGIN` connects an existing native observer as described
below. The launcher runs the compiler/API without `LOTUS_OBS`; observation belongs
to the application.

The native API can also be invoked directly:

```sh
/absolute/path/to/hale-api /absolute/path/to/dna-project 8792 "$PWD/iris/cockpit/web"
```

Open <http://127.0.0.1:8792/>. The API binds to loopback. Omitting the final
webroot argument preserves the API-only service. The webroot is this static asset
directory, not the DNA project or its Record. Only the eleven named assets and
the observer connection metadata described below are served; the service is
not a general file server.

The project chooses trusted local access or its existing OIDC configuration;
see [API identity and content](../../dna/api/README.md#identity-and-content).
The shell and assets contain no Record data and can load before sign-in. Every
data request still passes through the API's authentication boundary. The OIDC
callback must point to this service's `/auth/callback` on the same origin.

The existing API can be supplied from a Hale build or an upstream artifact.
Definitions and Knowledge need a service that advertises and implements their
read contracts. Unsupported connections retain explicit unavailable states.
This checkout launcher is separate from `hale iris`. Its project service is
the operator-machine head; there is still no `hale dna api` or `hale dna cockpit`
CLI subcommand, complete Compose profile or hosted deployment.

### Ordinary Hale application controls

The [generic application service](../service/README.md) serves the same shell
with Application and Runtime workspaces. The [intake-control example](../examples/intake-control/README.md)
provides a real application-owned mode control, durable receipts and a work loop
whose intake follows the committed mode. Build it using that example's SQLite
development prerequisites, then run two processes:

```sh
iris/examples/intake-control/intake-control run /tmp/intake.sqlite operator
iris/examples/intake-control/intake-control serve /tmp/intake.sqlite 8793 "$PWD/iris/cockpit/web" operator
```

Open <http://127.0.0.1:8793/>. The generic host selects `#/application`; this
connection needs no DNA Record or services. Choose an advertised mode, review
its captured application incarnation and revision, and explicitly submit the
change. Desired configuration, app-effective configuration and activity are
shown separately. The app determines authority and whether a command succeeded.

Before sending, the browser reserves a request identity and intent digest under
the application and principal, using Web Locks and localStorage. It saves no
draft body or requested value. A lost reply remains uncertain; reload or
Check request status recovers the same request through GET, without another POST.
Recovered receipt fields and their digest must match the saved intent binding.
The reservation must be explicitly released before another change is prepared.
Current state can be unavailable while receipt recovery remains available.

Reads have a five-second deadline and a 64 KiB response limit. State polls every
two seconds while visible, with one read in flight. Hiding the page or losing
application/principal identity clears current state, drafts and displayed receipts.
Saved recovery metadata stays scoped to its original application and principal.
The browser requires a secure context with Web Locks, randomUUID and SubtleCrypto
for submission; loopback HTTP qualifies in supported browsers.

The first profile supplies one control in an explicitly registered application
and uses a startup-configured trusted-local principal. This is an application
administration proof, not a DNA practice or workflow executor, hosted identity
system, arbitrary source editor or universal command store. Runtime observation
keeps its own explicit connection and never establishes command authority.

## Current surface

- **Projects:** when the project service answers behind the API, create,
  initialize or attach a DNA project on this machine and operate the attached
  one: sync and publish the Record, configure and sync the forge, start and
  stop the local body, preview or run body provisioning, drive a remote body,
  set or rotate secrets by source name, probe models with an explicit spend
  confirmation, propose and close connections, publish, accept and sync
  handoffs, and start or stop the observer. Every action is the CLI verb run by
  the head; the browser shows its durable receipt and reads the head again
  before it claims an effect.
- **Work:** recorded workflow executions and a separate **Handed Tasks** view.
  Handed Tasks show the existing responsibility, acceptance requirements and
  assignment history; an explicitly authorized local profile can reassign a
  supported open Task to an eligible person. A native command head also lets
  the signed-in principal raise work with the **New task** form, the
  cockpit's `hale dna ask`; the organism's answer is a separate, later fact.
- **Practices:** paged proposals and revisions, available document text,
  lifecycle, provenance, rationale, governing Review and superseded digest.
- **Reviews:** exact subject, required authority and recorded decision, linked
  back to the practice when one is identified. Approval and activation remain
  separate facts.
- **Organization:** source-backed static instance outline and inspector,
  explicit position groups, typed parameters and message/supervision contracts,
  source files, dependency fingerprints and separate declared ownership maps.
  Select an instance, then choose **Enter branch** to show its root and immediate
  children in Topology or Outline. **Whole organization** returns to the current
  page's overview. Breadcrumbs follow returned native parent relationships;
  parents outside the loaded page are explicit references, not inferred ancestry.
  A branch root outside the page is read by its exact identity at the same source
  snapshot. Its children remain limited to the current page: an empty page does
  not establish that the branch has no children. A structural root remains visible
  as context under **Declared positions**, with excluded children counted;
  **All structure** includes those children on the page.
  Branch, inspected instance, position filter and **Working locus** remain
  independent. Inspecting a child retains the branch; deep links and browser
  history preserve these choices. Changing application or working locus clears
  the branch, while refresh rechecks its identity against the source. Entering a
  branch changes only the source view, not the signed-in principal, working
  position or authority. Static declarations do not establish running occupants,
  effective grants or changes to the application.
- **Runtime:** connect explicitly to a configured native Iris observer and
  inspect observed processes, locus containment, topic shapes, process-to-process
  routes and counters in the cockpit. It requires no DNA connection or session.
  Observer identity remains separate from application/Record identity; this
  workspace neither invokes controls nor proxies the observer.
  Enter a process or locus to inspect its immediate observed contents, and use
  the containment breadcrumbs to move outward. Inspect opens the detail panel
  without changing that viewing scope; the accessible list uses the same scope.
- **Definitions:** when supplied by a compatible API, the application's native
  catalog with exact revisions, ordered Steps, leaf specifications, child workflows,
  reverse dependents and loaded-source provenance. Definitions are distinct from admitted
  executions. Catalog capture, validation and publication remain the service's
  responsibility; the browser neither discovers a catalog from source nor runs
  a workflow to populate this view.
- **Knowledge:** when advertised by a compatible service, inspect visible items,
  incoming/outgoing relationships, locus bindings and target relevance. A focused
  relationship map lets you navigate between connected items; it uses only the
  current response page, with no inferred edges or extra background graph reads.
  Select a relationship to inspect its exact identity and direction, including
  distinct relationships between the same endpoints and self references. Prepare
  removal of that relationship, revise the focused item or add a relationship in
  the same workspace. **Compare draft** marks the requested change while
  **Current graph** retains the native page; requested additions have no stored
  relationship identity. The visible-item picker and exact-identity input keep
  endpoint selection explicit, and review checks a referenced item against the
  same source and visibility snapshot. **Back to relationship map** and
  **Continue editing draft** preserve the active draft; choosing another
  contextual action does not replace it. Editing clears checked evidence, and
  navigation or source/access changes clear the draft. Review and download prepare
  a change only: no proposal, adoption or graph mutation is submitted. Page limits
  remain explicit, and the list and exact receipt identities remain available.
  Connections without this capability show it as unavailable. Acting-position
  permissions and administrative actions require their authoritative contracts.

Workspace URLs use browser hash routes and preserve opaque native IDs
through query encoding. Browser back/forward navigation works without requiring
a server route for every object.

The source footer identifies the inspected Record head and revision. Refresh
reads that local Record again; it does not fetch a remote. Paging holds the
original snapshot, and a 409 restarts the page sequence with a visible notice.
Missing objects, unavailable sources, empty catalogs and sign-in requirements
have distinct states. Old content is cleared when a request can no longer
establish readable data. Documents and drafts are not persisted in browser
storage; command recovery retains only the scoped request metadata described below,
and Projects keeps one separate slot per principal for its own request identity.

Receipt visibility remains the API's decision, including current Ledger policy
where applicable. The browser explains returned availability and provenance;
it cannot restore suppressed text or infer hidden relationships.

## Projects and the project service

`web/projects.js` (`window.IrisProjects`) is the Projects workspace. It talks
only to the four head paths under `/api/hale/v1/head`, validates their closed
envelope (`api_version`, `head.profile === "dna.head.v1"`, principal, active
project; an unexpected key anywhere is a different service) and never treats a
head answer as a Record envelope.

The shell probes `GET /api/hale/v1/head` once per page, before the first
Record read or the Projects view. A plain Record API answers 404: the shell
continues unchanged, the Projects entry stays hidden and nothing else is sent.
A head that answers `detached` lands the page on Projects, because there is no
Record to read; a head that answers `attached` keeps the requested view and
shows the Projects entry. Runtime never probes. Record reads refused with
`head_detached`, `head_api_unavailable` or `upstream_timeout` render their own
state cards with an **Open Projects** action.

The workspace shows the head's state, the registered projects with their
Record head, body, forge and recent receipts, the onboarding forms (create
pre-fills the head's `projects_dir`; `discover` is an explicit checkbox) and,
for the attached project, one form per operation. Forms the head reports
unavailable are disabled with the head's reason code; a busy head disables all
of them and names the running command. Values are checked in the browser
against the verbs' own grammars before anything is sent; an invalid value is
listed beside its field and no request leaves the page.

A submission reserves its identity first: one `localStorage` slot,
`iris.projects-recovery.v1:<principal>`, holding only `{version, request_id,
operation, target}`, taken under a Web Lock before the POST and cleared once
the receipt is terminal. A reload restores it as a GET lookup, never a POST.
The receipt lifecycle `queued → recorded → admitted|refused → running →
succeeded|failed|outcome_unknown` renders as stages with an inspector; a
non-terminal receipt is looked up every two seconds, and the head is re-read
every two seconds while its API child is starting. `data-observation` on the
request region becomes `observed` only after a fresh read of the head (and of
the attached project) shows the effect: a fresh active project, a moved Record
head, a running child, a stored secret name, a listed connection. An
`outcome_unknown` receipt names the command, its target and run, links the run
log and explains that nothing is retried automatically; there is no retry
control. Refusals decided by the head before it recorded anything (400/409)
release the slot; a lost response keeps it and is looked up until it settles.

Secret values never enter the browser. `Set secret` and `Rotate secret` take a
NAME (the credential names the head found in the project's model catalog, or a
typed one) and a SOURCE the head advertises: a one-line 0600 file under the
head's sources directory (`${XDG_CONFIG_HOME:-~/.config}/hale-dna/sources/<NAME>`)
or a variable exported in the head's environment. The head pipes that source
into `hale dna secret`; the form has no value field, and a request carrying one
is refused by the head.

## Native Runtime connection

Start your existing native observer independently. Configure its exact trusted
origin when starting the API's static shell:

```sh
HALE_IRIS_OBSERVER_ORIGIN=http://127.0.0.1:8787 \
  /absolute/path/to/hale-api /absolute/path/to/dna-project 8792 "$PWD/iris/cockpit/web"
```

Open `/#/runtime` and select **Connect observer**. Configuration allows that one
origin in Content Security Policy; it does not connect automatically. Use only
an `http://` or `https://` origin, with an optional port and no trailing slash,
path, credentials, query or fragment. Invalid nonempty configuration prevents
shell startup. The browser requests only `/snapshot`, omitting credentials and
referrers and rejecting redirects. A URL typed for any other origin prepares an
external link; it cannot expand the configured connection policy.

Containment navigation follows returned parent IDs. Missing parents remain
explicitly unobserved; an empty focused view means no immediate children were
observed, not that the application can never create them. Topic routes describe
the fleet's processes and retain their separate scope. New samples preserve a
still-observed focus; departures move outward with an explanation. A reported
process restart clears its local focus and selection. Disconnect, unavailable
observations and page suspension clear all local navigation. These controls do
not set a DNA acting position or establish an application incarnation.

An independent static host can serve `index.html`, `app.js`, `runtime.js`, `application.js`,
`definition-draft.js` and `styles.css`, plus `/iris/observer.json` containing exactly:

```json
{"profile":"hale.iris.observer.v0","origin":"http://127.0.0.1:8787"}
```

Use the same security headers as [WebAssets](../../dna/api/web.hl), setting
`connect-src 'self'` plus that exact configured origin. Serve the metadata with
`Cache-Control: no-store`. An empty origin disables in-cockpit observation while
retaining safe external links. The native observer must permit cross-origin
reads, as the existing Hale observer does. HTTPS browser policies still apply.
This standalone Runtime route makes no DNA API request and needs no Record,
database, body or organization process.

Polling uses one request at a time, a three-second deadline, a two-MiB response
limit and bounded collections. Failure, disconnection, navigation and hiding
the document clear observation/selection and stop polling. Reconnect is explicit.
An empty successful observation differs from an unavailable observer. Repeated
or regressing timestamps cannot establish fresh activity. Unsafe legacy numeric
measurements are shown as unavailable; unsafe identity numbers reject the
snapshot. Exact numeric validation requires JSON parse source context support;
browsers without it retain the external-link path.

Containment follows reported parent IDs. Missing parents/endpoints remain
unobserved. Topics retain both name and shape; name-only route references are
marked ambiguous when multiple shapes share a name. Matched routes are
observations, not proof of application effects. The observer's bounded buffers
and overruns limit coverage. PID and process-local locus IDs do not establish
restart-stable incarnation identity, and model hashes do not identify an
application. These values never become command targets or automatic DNA joins.

## Practice administration

Practices provides entry points to create a practice, edit its text and canonical
scope, manage additional applicability, and retire it. These open the existing
Knowledge editor in Practice context, with the selected document and graph kept
visible. They use the native node and binding command profiles advertised by the
composed service; selecting a working position does not grant mutation authority.

For an existing practice, the browser checks its exact document and graph item at
the same Record snapshot before opening a draft. Revision starts from the recorded
author and target, even when the operator is viewing another position. Proposed
changes remain drafts until explicitly reviewed and submitted. The resulting
request, independent Review and observed adoption or retirement remain separate
facts, using the existing saved-request identity and GET-only recovery.

A binding makes a practice relevant in a Knowledge context. It does not change
the application's existing global name-based acceptance-obligation rule. A
revision creates a successor and retains the predecessor and history; additional
bindings are not automatically copied to the successor. Retirement preserves the
earlier text and bindings as history. Pending and retired practices remain
inspectable without offering a new change against an inactive version.

## Practice text replacement profile

The standalone API does not provide this replacement profile, so its proposal action is disabled.
A composed application must advertise the exact `dna.practice.propose.v1`
profile, its availability and the current person's authorization before Iris
enables submission. Selecting an organization position does not grant that
authorization. This adapter currently supports replacing a readable, ratified,
current practice whose author and target are `org`; it does not edit the stored
document in place. Review decisions use a separate capability described below.

The editor pins the predecessor's exact identity, preserves literal text and
rationale, and shows both texts before submission. The request includes that
subject and the expected authenticated principal as preconditions. The service
compares the latter with its resolved identity before provider dispatch, so an
account switch cannot silently submit under a different person.

The proposal remains beside the selected practice. Its comparison highlights
separate additions and removals; **Read exact text** removes highlighting while
preserving the complete text, Unicode and captured line endings. Returning to
editing retains the unsent draft. Large changed regions use a bounded, grouped
comparison without truncating either document.

Before sending, Iris reserves a recovery identity under an exclusive browser
lock and verifies its local-storage write. The saved fields identify the
application, principal, request, operation/version, position, target and exact
subject; proposed text, rationale and returned receipt content remain in memory.
Browser storage and Web Locks
must be available to submit. Drafts clear when the view reloads or changes.
Concurrent tabs cannot overwrite that scoped unresolved request reservation.

An interrupted request is recovered by its original ID through authenticated
GET, including after a reload. Iris does not automatically resend a POST or
mint another ID. An unavailable or not-yet-found receipt retains the reservation.
The result presents proposal creation, exact-candidate Review and adoption as
separate facts. Manual status checks fetch current evidence; approval alone never
becomes an adoption claim. Completed command reservations can be explicitly
dismissed. Recovery access and all mutation authority remain service decisions.

The receipt opens in the selected practice or Review when its exact identities
match. Select Command, Proposal/This verdict, Review, or Adoption in the connected
outcome view to inspect each fact independently. Exact request/source identifiers
remain under the evidence disclosure. Checking status preserves the selected
outcome and uses GET only. When the associated document cannot be read, recovery
remains separately available without displaying its old document content.

The native provider must supply durable admission and recoverable domain
progression. The adapter and browser conformance fixtures do not establish those
guarantees. See the [service contract](../../dna/api/contract/v1/README.md).

## Exact-candidate Review decisions

Organization source Reviews also show a **Current responsibility check**. It is
separate from the source comparison and the Review-to-running journey: four
selectable categories expose current recorded Tasks, Work, schedules and
uncertain intents/effects without treating their overlapping counts as a total.
The browser joins the check to the exact Review, candidate and Record snapshot.
Refresh rereads them together; access loss or incomplete evidence clears counts.

This first reader covers a bounded local Record profile. Unsupported operational
history, protected references and prior Ledger adoption remain unavailable. The
check does not attribute work to changed positions, record what a reviewer saw
at decision time, or authorize source application. Those require the owning
service's position binding and apply-time admission barrier.

The independent `dna.review.verdict.v1` capability enables decisions on pending
practice-candidate Reviews. Iris fetches the canonical practice at the Review's
exact source snapshot and checks its digest, Review link, pending state and
organization-wide author/target before enabling confirmation. A question or an
authority label alone cannot authorize a decision or substitute for candidate
text. Protected, missing, inconsistent and already-settled candidates remain
unavailable for this action.
The first native practice-review profile also requires an explicit non-mutation
Review, no approver quorum, and its declared `board` authority shape. These facts
restrict eligibility; the separate command capability establishes permission.
Older responses missing those classification fields remain readable but cannot
enable a decision.

Choose Approve, Reject or Request revision, optionally add a literal decision
note, then confirm the exact candidate and choice before submitting. No choice
is preselected. The command names the Review and its distinct candidate digest,
expected principal and required pending state. The native provider must recheck
those conditions and current authority when the delayed decision is made.
Abstention, mutation Reviews and reapproval are outside this profile.

The result distinguishes this command's accepted/refused verdict, the Review's
overall settlement and the practice's adoption. A refused command can appear
beside a Review approved by another command; that approval does not turn the
refused request into success. Accepted rejection/revision is command success,
with no invented adoption outcome.

These operations share one recovery reservation per application/principal with
Organization and Task commands. Ordinary Practice/Review metadata uses version2;
legacy version1 practice metadata remains readable without
rewriting or deleting it. The original storage key and lock are retained so old
and new tabs cannot reserve independent slots. Unknown metadata stays blocked.
Dismiss a validated completed request explicitly before starting the next one;
Iris then refreshes domain reads before checking eligibility for another action.
404 and unavailable lookup cannot release an unresolved reservation. Recovery
remains available during unrelated collection failures and write revocation.

## Definition drafts and native validation

Select an exact workflow revision to enter its ordered Step bands and member
nodes. Inspect a leaf's full Work specification or enter an exact child revision;
the traversed path identifies the Step and member that led there. Shared children
retain multiple possible callers. Direct catalog references and source claims are
available beneath the working surface.

`Draft next revision` copies the selected revision into browser memory. Edit the
title, add or reorder Steps, and edit leaf or child members. Revisions and numeric
fields remain exact decimal strings. Existing definitions and their pinned child
references remain unchanged. Navigating away, refreshing, switching application
or losing access discards the draft.

The optional `dna.definition.draft.v1` capability permits an explicit native
validation request. It does not enable domain writes. The Hale operations layer
compares the exact catalog basis, retains all old revisions, appends the proposed
N+1 and uses the existing catalog rules to check every root. Request and catalog
resource limits are separate from semantic validity. The current profile allows
8192 UTF-8 bytes per workflow/member text field and 16384 across the complete
candidate catalog; these protect native codec allocation and are distinct from
the workflow admission policy. Any edit invalidates the
previous result; obsolete responses cannot restore it.

A valid result provides a digest-bound generated Hale registration fragment for
the complete candidate catalog. The fragment relies on the containing seed's
explicit DNA core import and registers into a fresh catalog. It is an export of
loaded definitions, not a reconstruction or replacement of the module named in
source provenance. Original source ownership, source/structured roundtrip,
governed publication and activation require their own provider. No publication
or running-state claim follows from successful catalog validation.

## Development and verification

Edit `web/index.html`, `web/styles.css`, `web/runtime.js`, `web/application.js`,
`web/definition-draft.js`, `web/organization-draft.js`, `web/knowledge-draft.js`, `web/task-administration.js`, `web/projects.js`, `web/task-create.js` and `web/app.js`, restart the API to load
the changed assets, then reload the browser.
There are no external scripts, fonts or asset services. JavaScript renders
native content as text. A connection without a compatible command provider cannot submit domain changes.
A separately advertised definition-draft validator accepts explicit, nonpersistent
validation requests; supported proposals and Review decisions require deliberate
submission.

Browser interaction tests live in `tests/`. DNA cases use a temporary Git Record
populated by native Hale writers and the real API; focused response overrides
exercise error and reconnect paths. Node/Playwright are test tooling only.
Runtime cases use an independent static host; the optional native gate attaches
the real observer to a plain Hale application with isolated shared memory.
Native HTTP, authentication, engine and persistence verification belongs to
the corresponding upstream implementation.

From the repository root, with a compiler and API already available:

```sh
export HALE_BIN="/absolute/path/to/hale"
export HALE_API_BIN="/absolute/path/to/hale-api"
cd iris/cockpit
npm ci
npx playwright install chromium --only-shell
npm test
```

Set `HALE_BIN` to another current compiler if necessary. Linux environments may
need Playwright's browser system dependencies (`npx playwright install --with-deps
chromium --only-shell`); CI installs those and runs this suite in partition 1.
The browser tests exercise a real 404, receipt redaction and snapshot conflict;
controlled HTTP responses cover lost authentication and source unavailability.
The optional Definitions lane consumes a supplied application-catalog fixture
through the API startup and authentication path. It checks recursive references,
revisions beyond JavaScript integer precision, unavailable/invalid providers,
catalog changes, mobile detail and history. Linux native fixture processes have
hard memory and CPU limits so
an allocation regression cannot exhaust the browser test host.

The browser launcher builds only its baseline Record fixture. Definitions and
Knowledge integration require explicit compatible provider binaries; their cases
are visibly skipped otherwise. These optional gates exercise the browser and
API contract, not the upstream execution or persistence suites. See
[browser test setup](tests/README.md) for the executable fixture interfaces and
commands. Provider binaries may come from upstream or a separately prepared
integration build; their source need not be present in this browser change.

The complete administration scope is defined in
[#690](https://github.com/hale-lang/hale/issues/690). Browser reads
are one part of that scope; they do not establish completion of the governed
editing, command recovery or operational proof loops.

Organization source preparation is available when the host advertises
`dna.organization.draft.v1`. Select an instance in the chart and choose **Edit
this instance**, or open **Edit organization** below the chart. The structured
form prepares child additions, moves between parents, instance-name changes, position-group membership,
literal Int/Bool/String declaration defaults and source-field removal. It lists
shared source instances and the full removed subtree before preparing changes.
Removing the last position member requires an explicit choice to allow an empty
group; the editor never silently weakens that source rule.

**Move to another parent** previews the source subtree and every destination
instance before validation. Moving a field between shared declarations can
change the number of instances. The complete constructor moves with the field,
including explicit settings and inline comments. Changing the destination or
child name clears the review acknowledgement. Existing ownership declarations
and live obligations are not reassigned by a source move.

Each form changes exact spans in the captured module and immediately requests
native whole-organization validation. It preserves neighboring code, comments,
methods and explicit constructor overrides. The updated chart and export come
from the native response. Handwritten source edits invalidate structured bindings
until checked again. Imported declarations, computed defaults and unsupported
source constructions remain in the source editor without guessed bindings.
Position membership does not establish an occupant, ownership map or authority.

After validation, **Compare changes** shows the current and checked candidate
instances together, including marked outlines for removals. **Current source**
and **Checked candidate** show either projection while retaining the same node
positions. Nodes and containment edges come only from the exact native before
and after responses; a renamed path remains a removed identity and an added
identity. Select a node to compare its parent, declaration and role side by side.
**Enter this branch** focuses its descendants, and **Whole organization** returns
to the complete chart. **Edit candidate instance** selects that instance in the
structured editor; it is disabled for removed instances. These viewing controls
neither request more data nor change the source.

For direct source editing, open **Edit organization source** below the chart,
edit the captured Hale module, inspect its source diff, and select **Validate
organization**. **Download validated Hale** preserves the exact edited bytes;
validation evidence records the principal and original source/dependency/Record
basis. Further edits invalidate the checked chart, its instance actions and the
export. Navigation, access changes and stale source clear the draft. Nothing is
persisted in browser storage. This editor prepares source; it does not publish a
proposal or change the running application.

**Edit ownership** below the declared ownership map prepares assignments,
owner membership and the hosting party in the captured `dna/org/owners` file.
Forms preserve neighboring entries, comments and line endings. The native DNA
ownership model previews inherited owners after an assignment is removed,
changes between shared/single-owner mode, and affected owners and review members.
Missing review members remain visible. Changing this draft does not replace the
current viewing-context list or grant session authority. **Download validated
ownership map** exports the exact checked source; publishing and observing its
activation remain part of the governed source-change integration.


### Knowledge changes

**Prepare knowledge change** opens the editor against the current visible
Knowledge snapshot. With no item selected it prepares a new item. With an item
selected it also prepares a revision, retirement, directed relationship addition
or removal, or locus binding addition or removal. Revisions retain the exact
predecessor identity; removals select an exact relationship or binding from the
loaded page. The service must determine authority, binding class, supported edit
semantics, review requirements, and adoption.

**Review knowledge draft** rechecks the authenticated principal and rereads the
same native Knowledge snapshot, including the selected item, loaded relationship
and binding pages, and any proposed relationship endpoint. Changed source,
visibility or identity clears the draft. A missing/protected endpoint stays
unavailable. Connection failure retains editable text without a reviewed export;
edits and navigation invalidate previous review evidence. No draft text enters
browser storage.

With the native Knowledge command provider configured, **Submit knowledge
change** uses the capability returned for the selected operation. Node and
binding proposals follow exact native Review and adoption or refusal.
Relationship link/unlink uses direct or reviewed admission according to policy.
For reviewed relationships, connected proposal, Review, effect and graph stages
keep an approval separate from the observed result. The Review shows the exact
directed candidate and preserves proposer independence. An intervening tuple
change can leave the Review approved while the native effect is refused.

The browser saves only request recovery metadata. Reload and **Check relationship
request** recover the original receipt without resubmitting; its durable variant
determines whether the request was direct or reviewed, even after a policy mode
change. Graph confirmation uses a fresh service snapshot. Removal requires
complete relevant pagination and a final visible receipt at the same source
basis; a missing row on one page is not proof of removal. No optimistic graph
change is presented as an observed native effect.

Services without the required command capability retain preparation and
**Download knowledge draft**. The exported `iris.knowledge.change-draft.v1`
artifact is not a native command, receipt or accepted proposal. Its
`source_checked` flag means only that the browser rechecked visible service
reads; `submitted` remains false. Impact shows the loaded pages and explicitly
names missing run/Definition/Practice dependencies. The current native reference
composition uses local fixed-policy Record authority and a memory graph; this
does not establish routed, hosted, PostgreSQL or sustained-service acceptance.

### Shared organizational context

**Working locus** carries a declared ownership scope between Organization,
Knowledge, Practices, Definitions and Reviews. The choices come from the native
Organization ownership map. **Work from …** beside that map selects a scope;
deep links and browser history preserve it. The picker exposes the checked source
commit and declared owner. It does not bind compiler instance paths to domain
positions, establish an occupant, impersonate an owner or grant command authority.

Knowledge applies the scope through its native target-relevance query, including
applicable ancestor bindings. Practices and Definitions keep their native pages
unfiltered and mark exact targets on that page; Definition matches cover direct
leaf targets only. Reviews retain their recorded scope and required authority.
Ordinary Application and Runtime views remain independent of this DNA context.

Changing context clears unsaved drafts and restarts pagination. An independently
entered Knowledge relevance filter clears a different shared context. Removed or
unavailable selected scopes stop the workspace read until the user chooses or
clears the context; they never silently broaden the view. Context and workspace
must use the same Record snapshot. Authentication failure clears context metadata.

### Recorded Work

**Work / Executions** reads workflow admissions, bound recipes and native `WorkflowProjection`
states through `/dna/workflows`. Open a run, move through its ordered Steps,
enter an exact child task, or inspect a Work's attempts and accepted result.
The open execution keeps ordered Step bands beside a focused inspector. Follow
a completion barrier into its exact member, return to the child's spawning Step,
or select an attempt in its recorded history. Attempt links preserve exact
identity through reload and browser history. Results lead; bound specifications
and full identifiers remain available on demand. Narrow screens retain explicit
return controls and keyboard focus between the workflow and its selected object.
Barrier keys resolve only within their owning Step; unavailable or ambiguous
members receive no invented destination. A pending key can name a failed or
cancelled member, and does not by itself establish an actionable obligation.
The captured recipe retains the definition revision, target, requirements and
opaque knowledge-binding specification. Definition links select that exact
revision; this reader does not substitute the current definition or graph.

Bound future members, registered Steps, activated Steps, outstanding attempts,
attempt outcomes, Work settlement and committed Step completion remain distinct.
A member result does not advance a Step in the browser. Cancellation can leave
admitted responsibilities outstanding. A task birth without a wf1 admission or
refusal is unclassified, not proof of a particular engine or completed run.
History records native projection order; it does not invent times or runtime joins.

The initial source is the trusted-local Record before Ledger adoption. Hosted
identities and applications that have adopted a Ledger require an owning-service
execution reader; this adapter reports that gap instead of returning an empty or
partial run list. Protected work classes and restricted or redacted receipt references
suppress the whole affected run from this reader, including its identifiers and
counts. Snapshot movement and authentication loss clear stale details. This execution view
is inspection only: starting, retrying, cancelling, satisfying obligations and
recovering live execution remain service-owned integrations.

### Handed Task administration

Open **Work / Handed Tasks** to inspect native Tasks with a recorded human
handoff. The outcome and current assignee lead; the same Task's obligation,
bound acceptance, evidence requirements, waiting reason and selectable assignment
trail remain available. Reassignment preserves these responsibilities and the
Task's open state. It does not complete the Task or remove a position.

In **Organization / Declared members**, select a person's **Inspect recorded
assignments** control to open Tasks with that exact recorded assignee. The same
name under different declared owners remains a separate source entry, but each
opens the same exact-person query. This does not identify the owner of those
Tasks, a position occupant or a permission grant. It clears the working locus
rather than treating source ownership as an assignment filter.

The service filters before pagination. The Task list preserves the assignee
through page navigation, reload and browser history; **All assignees** clears it,
and switching application clears it too. The returned filter and every listed
assignee must match the requested identity. Counts cover visible recorded Tasks
in that scope, including retained terminal handoffs; they are not a complete
inventory of a person's obligations across workflows, intents or schedules.
Unavailable or mismatched reads do not become an empty responsibility list.

With an explicit Task command grant, choose an eligible **New assignee**, then
**Review reassignment** and **Confirm reassignment**. The recipient choices come
from the native capability; the browser cannot invent people or infer authority
from the working locus. The service rechecks the exact Task assignment digest,
current assignee and both people's eligibility before recording the change.
Transferred handoffs remain inspectable without this action; incomplete or
unsupported native history makes the reader unavailable. Terminal handoffs
retain their history and cannot be reassigned.

The browser saves version5 scoped request identity before submission in the
existing shared command recovery slot, without storing the assignee payload or Task text.
An unresolved request blocks a different submission. After a lost response or
reload, **Check request status** uses GET only; it never automatically repeats POST.
The command receipt and a fresh read of the Task are separate evidence: a
recorded reassignment need not yet be observed in the current detail view, and
a later assignment or terminal state does not erase the earlier receipt.

When reassignment moves the selected Task out of the original person's list,
its inspector remains open through an independent exact-ID read at the same
fresh Record snapshot. The new assignee and preserved assignment trail remain
visible alongside the original list context. A Task leaving that list is not
completion, deletion or proof that the person has no other responsibilities.

This source-built profile uses a trusted-local, Record-only application and an
explicit startup `HALE_DNA_TASK_POLICY`. The default API remains read-only;
use the [composed command head and Task policy](../../dna/api/README.md#handed-tasks-and-reassignment)
to enable the action. Task reads are current snapshots, not historical browsing;
a stale snapshot requires a refresh. Restricted or unavailable evidence clears
the affected details. Person retirement, cross-owner transfers, hosted or Ledger
administration, and joins from declared source ownership to live responsibility
are outside this profile. The policy's owner label is not such a join.

### New task

On **Work / Handed Tasks**, a native command head shows the **New tasks
enabled** badge and a **New task** form: an outcome (**What should happen**,
1..8192 bytes) and a locus (**For locus**), with the whole organization
(`org`) offered first and the working context's declared loci after it, as
`hale dna ask` without `--to` addresses the organization. **Review new task**
shows the exact ask in the signed-in principal's name; **Confirm new task**
submits it once. There is no separate grant: the authority is the
authenticated principal, as with the CLI.

The receipt records the ask, not its answer. The service appends the same
`intent.requested` row the CLI writes, at the Record head the form was
prepared against (a moved head refuses the request, and the form must be
prepared again). Whether the locus is this organization's to admit is the
organism's judgment, recorded separately; **Check request status** re-reads it
through GET only and shows the intent as `requested`, `offered`, `refused` or
`born`, naming the Task once it is born. A born Task joins the handed Tasks
only after the leader hands it to a person, so the request panel is where a
raised ask is followed until then.

Recovery metadata uses version 7 in the existing shared request slot, holding
the request identity and the prepared Record head; no outcome text is
persisted. An unresolved request blocks a different submission. See
[Raising work](../../dna/api/README.md#raising-work) for the row, the wire
shape and the receipt.

# Person responsibility and retirement

Selecting a declared member opens their exact recorded assignments. When the
native person capability is configured, the unselected Task inspector shows the
complete retirement plan: person, successor, and linked responsibilities whose
requirements will be retained. Retirement requires a separate explicit grant,
a reviewed exact plan, and confirmation. It preserves work and history; it does
not remove Organization membership or a declared position.

The result follows the saved request through native admission and a fresh person
read containing the exact retirement event. A lost reply or service restart uses
GET-only recovery, including after the write grant is revoked. Recovery metadata
uses version 6 in the existing shared request slot; no successor or Task content
is persisted. Unavailable or incomplete responsibility evidence disables the
retirement action.
