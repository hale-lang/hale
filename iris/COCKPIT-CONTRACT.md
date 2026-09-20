# Iris cockpit API contract — draft 0.1

Design draft for [#690](https://github.com/hale-lang/hale/issues/690).
This document proposes the browser/service boundary; it does **not** describe
implemented HTTP routes or freeze DNA's internal types. The issue defines
the broader product scope.
Implemented reads are documented in [dna/api](../dna/api/README.md), with
executable schemas in `dna/api/contract/v1`; the [browser documentation](cockpit/README.md)
describes their presentation. Broader routes and commands below are design
requirements, not a declaration that a service exposes them.

The branch also implements the deliberately smaller `hale.application.v1`
[generic service profile](service/README.md). It registers an application-owned
control provider, serves captured state and capabilities, accepts one guarded
enum change and recovers an exact request receipt. The [plain Hale intake example](examples/intake-control/README.md)
owns its SQLite configuration, authority and durable outcomes; Iris has no
universal command database. This implementation does not imply the broader
runtime joins, contextual authority or DNA operations below are available.

The branch's `/dna/organization` read endpoint projects checked static Structure
with its own source/dependency/artifact basis. It does not implement the broader
semantic `/dna/positions` or viewing/acting permission contract proposed below.

## 1. Ownership

Iris is an independently built browser application. The public API belongs to
Hale/DNA services and is shared with remote CLI clients. Its DNA adapter may be
served by an evolved head and composed with the native Iris collector.
Separate deployment does not require a new database or message broker.

| Component | Owns |
| --- | --- |
| Browser | Navigation, active viewing position, visual layout, unsent drafts, presentation |
| Hale/DNA API head | Authenticated sessions, scoped projections, typed operation dispatch, request recovery |
| Domain services | Canonical definitions, admission, policy, execution, durable facts, command outcomes |
| Native observer | Attachment to running Hale processes and observation snapshots/events |

The head adapts existing operations. It must not acquire a second workflow
engine, policy interpreter or writable copy of the knowledge graph. Durable
command tracking belongs in the existing authoritative request/receipt path;
an in-memory HTTP request table alone cannot provide restart recovery.

The browser receives neither database credentials nor direct ledger-append,
lease-mutation or arbitrary bus-publication access. Observation remains
independent of command availability and cannot backpressure the application.

## 2. Generic surface and DNA adapter

Proposed prefix: `/api/hale/v1`. Existing `/snapshot`, `/events` and DNA
`/api/*` routes remain legacy surfaces until an adapter explicitly supports
this contract. Their existence does not advertise v1 support.

The generic API identifies applications, runtime observations, relationships,
available operations, commands and their outcomes. DNA contributes position,
knowledge, practice, definition and execution resources. Ordinary Hale apps
must work without DNA services or invented organization data.

The response model describes entities and typed relationships, not screens or
canvas coordinates. The same data must support a spatial view, an outline or
a table. Keep runtime ownership, organizational responsibility, workflow
dependency and authorization relationships distinct. A layout change has no
domain effect unless the person deliberately issues a supported command.

### Proposed resource families

Paths below are relative to `/api/hale/v1`. They are the initial design
inventory; exact operation schemas are introduced with their adapter and
conformance fixtures, before a UI advertises them.

| Route | Purpose |
| --- | --- |
| `GET /applications` | Accessible applications, environment/record identity and adapter versions |
| `GET /applications/{app}/capabilities` | Implemented query/command contracts, their versions and service availability |
| `GET /applications/{app}/context?position_id=…` | Person, selected position, permitted contexts and effective contextual capabilities |
| `GET /applications/{app}/runtime` | Versioned observer projection, coverage, freshness and declared/observed identities |
| `GET /applications/{app}/dna/positions` | Scoped positions and explicit responsibility/ownership relationships |
| `GET /applications/{app}/dna/knowledge` | Bounded graph neighborhood, ideas, bindings and provenance |
| `GET /applications/{app}/dna/practices` | Practice versions, applicability and proposal/review lineage |
| `GET /applications/{app}/dna/definitions` | Definition catalog with exact revisions and references |
| `GET /applications/{app}/dna/executions` | Bound recipes, admitted work, projected state and links to facts |
| `GET /applications/{app}/activity` | Scoped durable history with causal references and coverage |
| `POST /applications/{app}/commands` | Submit one versioned, advertised domain operation |
| `GET /applications/{app}/commands/{command_id}` | Recover the command and authoritative result |
| `GET /applications/{app}/commands?request_id=…` | Recover when the original HTTP response, including command id, was lost |
| `GET /applications/{app}/events` | Authorized change notifications with resumable positions |

DNA collection reads accept an optional native `id` query parameter for an
exact object, plus explicit context and paging parameters. Native ids are
opaque strings. In particular, slashes in position and workflow ids are
preserved and URI-encoded in query parameters, never stripped or interpreted
as authority. The server returns related ids; the browser does not construct
child ids by parsing their current spelling.

No family implies generic CRUD permission. Each edit uses a named domain
operation with a typed input and declared completion condition. Unsupported
operations remain unavailable until their owning service supplies them.

## 3. Context, identity and evidence

An application descriptor establishes its environment and canonical record
identity. A request selects an application and, for DNA, a viewing position.
The response reports the authenticated person separately. The server resolves
the person from the session; a browser-supplied `as`, role or grant is not
authentication. Local trusted-operator and hosted authenticated modes are
explicit deployment modes, not interchangeable claims in a request body.

Viewing a position grants no permission to act from it. The context response
distinguishes positions that can be viewed from positions that can be used
for action. Every command rechecks current authority and records the person
and, when its adapter defines organizational context, the acting position.
Plain Hale applications omit that position rather than inventing one. The
visible capability list is guidance, not an
authorization token. Changing position invalidates the browser's scoped
caches and subscriptions; requests still in flight cannot populate the new
position's view accidentally.

Every resource reference contains `application_id`, `kind` and `id`.
Revisions, candidate digests, runtime instance identities and observation
positions are separate fields. A pid, entity name, journal sequence or
workflow definition revision is not a globally unique application identity.

Structured reads carry:

- `schema_version`: the response contract, independent of domain versions;
- `context`: selected scope, server-resolved person, and position when applicable;
- `data`: the typed resource or collection;
- `basis`: named source watermarks and exact revisions/digests used;
- `freshness`: current, stale, historical or unknown, with a reason;
- `coverage`: complete, partial or unavailable for the **authorized query**;
- `next_cursor`: opaque continuation when the collection is paged.

Per-source watermarks are required when combining Record, Ledger, knowledge
and runtime. Such a response does not imply one global atomic snapshot.
The server defines a consistent paging basis or explicitly invalidates an
expired cursor; it must not silently splice pages from different revisions.
Counts and omission explanations must not disclose inaccessible objects.
An unavailable source is not an empty collection or a zero measurement.

An object may expose a revision precondition for editing, evidence references
and explicit relationship kinds. Redaction happens before serialization.
Timestamps are supplied only when their source provides them; event time,
observation time and projection time are distinct. Sequence numbers cannot
be used to invent durations or a total order across independent memories.
Integers that may exceed JavaScript's exact range travel as decimal strings.

## 4. Commands and their receipts

Each advertised operation has an id and version, input schema, supported
target kinds, required subject preconditions, and a documented meaning of
success. Schemas and enums are versioned contract artifacts, not inferred
from a CLI help string. Domain arguments remain operation-specific; Iris
does not prescribe an organization's procedures.

The durable command profile below is required for DNA administration. A generic
Hale capability provider supplies its own authoritative state and recovery;
it need not use Record or Ledger. A provider offering only transient controls
advertises a separate, explicitly limited profile and cannot claim durable
`recorded` receipts or restart recovery. Iris shows that declared limit rather
than treating a successful publish as completion.

Illustrative envelope (the operation name is proposed, not shipped):

```json
{
  "request_id": "iris-request-7b82",
  "operation": "dna.practice.propose",
  "operation_version": "1",
  "context": {
    "application_id": "operations-production",
    "position_id": "org"
  },
  "target": {
    "application_id": "operations-production",
    "kind": "dna.practice",
    "id": "practice/supplier-identity"
  },
  "preconditions": {
    "subject_digest": "opaque-digest-returned-by-the-service"
  },
  "arguments": {
    "text": "Verify the supplied identity against its cited registry evidence.",
    "rationale": "Make the evidence requirement explicit."
  }
}
```

The server assigns a durable `command_id` and stores the authenticated
principal, acting position when applicable, canonical request and relevant subject versions
with it. Scope in the envelope must agree with the route and target.

`request_id` is generated before submission and reused for delivery retries.
Its deduplication scope is the authenticated principal and application. The
same id and semantically identical validated request return the existing
command; the same id with another operation, scope, argument or precondition
returns a conflict. An adapter must forward a stable domain request identity
or atomically associate it with the durable domain request. Calling today's
CLI again and allowing it to mint another id is not deduplication.

Before transmission, the browser persists the request id and its application/
principal binding. It retains that recovery metadata through ambiguous delivery
and reload until the existing command is recovered or explicitly expires.
It must not mint a new id merely because the command id or HTTP response was
lost. Sensitive command arguments need not be persisted in browser storage;
recovery metadata is sufficient to look up the existing request and grants
no authority by itself.

Request recovery must survive browser/head restart. Retention/expiry is
advertised: an expired recovery identity returns an explicit unavailable or
expired result, never permission to execute it again blindly. Lookup by a
request id is scoped to the authenticated principal and rechecks access.

The receipt distinguishes:

| State | What it proves |
| --- | --- |
| `recorded` | The authoritative request path durably knows the request |
| `admitted` | The domain admitted this operation under its current rules |
| `running` | Authoritative evidence establishes execution is in progress |
| `succeeded` | The operation's declared completion condition is evidenced |
| `refused` | Domain admission refused it, with a structured reason |
| `failed` | The admitted operation failed, with its known effects stated |
| `outcome_unknown` | The effect or result cannot currently be established |

Operations can skip inapplicable intermediate states. Local queuing and an
HTTP request being sent are browser transport states, not durable receipts.
An unknown outcome can later be reconciled on the same command, with new
evidence; it must not be rendered as failure or success in the meantime.
Receipts include resulting entity/evidence references and any supported
reconciliation action. No generic retry may duplicate an uncertain effect.

The completion condition matters. A successful `practice.propose` means the
proposal was durably created; it does not mean a reviewer ratified it.
Proposal, review and activation are separate domain objects/states.

Before admission, invalid schemas, absent sessions, forbidden scopes and
failed preconditions produce structured non-2xx errors. A recorded request
can return HTTP 202 with its current receipt and `Location`. Transport
acceptance never implies admission or completion. Error bodies carry a
stable code, human explanation, request identity when known, and authorized
conflict details. Examples include `unsupported_capability`,
`invalid_context`, `stale_subject`, `request_conflict`, `source_unavailable`
and `cursor_expired`. A CLI failure must not become HTTP 200 success text.

Subject/version checks and authority checks occur in the authoritative
operation's admission path, atomically with the decision where required.
A review applies to the exact digest shown to the reviewer; it must not
silently adopt whichever candidate happens to be current on submission.
Policies determine when review is necessary; the UI adds no universal gate.

## 5. Recursive workflow adapter

The pending `wf1` work gives Iris a useful semantic contract now. Map it to
structured browser data through an adapter, keeping these distinctions:

| Domain concept | Browser contract |
| --- | --- |
| Definition | Id, exact revision, ordered steps, leaf or child members, exact child definition references |
| Admitted recipe | Immutable expansion and applied admission limits; preserved even after definitions change |
| Task | Execution identity and definition binding; parent Task, spawning Step and member key when nested |
| Step | Ordered index, registered members by key/kind/entity, activation and committed disposition |
| Work | Logical leaf identity, allowance, current attempt, committed disposition and accepted result |
| Attempt | Attempt identity/number, request, performer, outcome and evidence; history from facts |
| Fact | Native fact identity/version and causal transition reference, with scoped source position |

`engine: wf1`, event codec version, definition revision and Iris API version
are independent. An explicit admission marker identifies an admitted `wf1`
execution; a `workflow.refused` fact also carries its engine and is presented
as a refused admission, not an execution that ran. Do not identify an engine
by a locus type name or the spelling `/wf1` inside an id. Legacy Tasks retain
a distinct adapter and must not be presented as fully observed recursive
executions.

The expanded recipe includes future members. **Bound is not admitted or
dispatched.** An attempt reporting `done` is not itself Work settlement. All
required members being joined is not a `step.completed` fact. A failed Step
or cancelled execution may still have outstanding responsibilities draining;
final workflow failure is recorded only after those responsibilities settle.
`pending()` in the current projection means not yet successfully joined;
it must not be relabeled as the set of running members.

Expose plan membership, admitted lifecycle and outstanding responsibilities
separately. Normalize internal space-delimited sets/maps into arrays while
preserving their identities and kinds. Preserve unknown state and missing
evidence. No invented percent-complete, timestamps or inferred success.

Card 06's projection retains the latest attempt; the history API must read
facts to expose earlier attempts. Replay remains read-only: viewing history
cannot dispatch an attempt or reproduce an external effect.

Definition editing and execution availability are separate capabilities.
A catalog's encode/decode support does not prove that publishing, adoption,
execution or live migration exists. Editing code-authored definitions must
use the authoritative source/change path until a domain-owned alternative
exists. A browser catalog must never silently become that alternative.

## 6. Updates and compatibility

Begin with ordinary JSON reads and polling where necessary. Advertise SSE
only when the adapter can honor its delivery/recovery contract. Durable
domain change notifications and lossy runtime samples are separate stream
classes; the existing observation ring does not become a durable audit log.

An SSE event carries a contract version, stream identity, opaque resume
cursor, affected resource references and their new version/basis. Durable
streams tolerate duplicates: clients apply versions idempotently. Loss or
an expired cursor requires an explicit reset/refetch indication. A runtime
stream reports dropped observations and coverage instead of promising
replay it cannot supply. New data is always authorized for the active
session/context; revocation must stop disclosure on an existing connection.

Capabilities distinguish implemented support, current availability and
effective authority. They are not guessed from build SHA or route presence.
For example, a disconnected supported service differs from an unsupported
definition mutation. The first response exposes these distinctions so the
UI can explain limitations honestly.

Breaking changes to identities, field meaning, enum meaning or command
semantics require a new contract version. Additive fields may be ignored by
an older client. An unknown execution state or operation version cannot
default to an actionable/success state. Version operation inputs strictly;
reject unknown mutation fields rather than silently dropping intent.

During development this draft can change deliberately with its fixtures.
Once a version is implemented, DNA internals may change behind its adapter
provided the browser contract's conformance cases still pass. The UI depends
on that contract, not on row layouts, process placement or CLI formatting.

## 7. Conformance requirements

Machine-readable schemas and representative fixtures must accompany each
supported contract. Illustrative fixtures identify themselves and expose
unavailable capabilities; they cannot establish live integration success.
Conformance includes ordinary Hale without DNA and the four DNA model workspaces.
Adapters must preserve domain-established authorship scope and expose publication,
adoption and execution only through their authoritative lifecycle contracts.

Supported command adapters must demonstrate: inadequate authority refused;
position viewing cannot impersonate its occupant; slash-bearing ids preserved; exact-subject review
and competing supersession; duplicate request recovery after a lost reply;
same request id/different content refused; failure and unknown effect shown
truthfully; unavailable sources distinguished from empty; and ordinary Hale
use without DNA. Hosted mutations also need the deployment's session, origin
and CSRF protections rather than relying on the local observer's trust model.

Each adapter must specify its durable request-to-command mapping and retention,
source-watermark/cursor formats, authoritative session-to-position capability
projection and operation schemas. Code-authored definition changes require
explicit proposal, validation and adoption semantics. The frontend consumes
these contracts and cannot invent their authoritative meaning.
