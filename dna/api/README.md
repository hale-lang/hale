# DNA read API

The service API reads practices, reviews and handed Tasks from an existing local DNA Record
and inspects the project's committed organization source. An application-composed
head can also expose its real workflow catalog. Configured Knowledge reads open
memory (Postgres) in the API's own process, as the record's head role, and read
the graph the body's spine projects there. The API runs separately from the
organization body and iris. Record and source reads do not require Postgres;
configured Knowledge reads require memory.
Typed projections in `dna/operations` are shared with the native CLI. The API
does not invoke CLI operational commands or parse terminal rendering. Organization
inspection uses the native compiler's machine-readable topology export.

This is an experimental source-built service. An optional
[face](../face/README.md) browses these reads from the same
origin. The checkout provides a [face launcher](../face/README.md#run-locally);
there is no installed `hale dna api` subcommand or Compose service profile yet.
Product scope and remaining service work are tracked in
[#690](https://github.com/hale-lang/hale/issues/690).

## Run

For the browser and the services together, `./dna/face/start.sh [PROJECT]`
builds this checkout's [project service](#head-project-service) (the head) and
its per-project native API (`dna/api/practice_review`) in temporary storage and
serves the ten browser assets from the head. The project is optional: without it
the head starts detached and the browser's Projects workspace creates,
initializes or attaches one. Use `--api BINARY` for an existing
application-composed API and `--head BINARY` for a built head. The launcher
inherits private service configuration; the head is trusted-local and refuses
a project configured for OIDC. See the face's README for fresh-project source
capture and service configuration.

From the Hale source checkout, using a current Hale compiler:

```sh
hale build dna/api
./dna/api/api /absolute/path/to/a/dna-project 8792
```

The project must already have a DNA Record (`hale dna new` / `init`). The server
binds only `127.0.0.1`. Stop it with Ctrl-C. Discover the native application id:

```sh
curl http://127.0.0.1:8792/api/hale/v1/applications
```

To serve the face as well, pass its static directory as the third argument:

```sh
./dna/api/api /absolute/path/to/a/dna-project 8792 "$PWD/dna/face/web"
```

Open <http://127.0.0.1:8792/>. The asset whitelist is `/`, `/app.js`, `/application.js`,
`/definition-draft.js`, `/organization-draft.js`, `/knowledge-draft.js`, `/task-administration.js`, `/projects.js`, `/task-create.js` and `/styles.css`.
URLs never become filesystem paths. All ten assets must exist and be nonempty
at startup. They are loaded once, so restart after changing them. API-only mode
retains its existing routes. No legacy mutation routes are enabled in either
mode. Browser assets and authentication share the API origin; there is no
cross-origin API contract, and the CSP's `connect-src` is the API's own origin
only.

Use the returned id in the following routes:

| GET route | Result |
| --- | --- |
| `/api/hale/v1/applications/{id}/capabilities` | Principal mode, implemented reads and the head's socket transport |
| `/api/hale/v1/applications/{id}/dna/practices` | Named practice proposals, lifecycle, attribution and available canonical text |
| `/api/hale/v1/applications/{id}/dna/reviews` | Review identity, exact subject, required authority and recorded decision |
| `/api/hale/v1/applications/{id}/dna/tasks` | Current handed Tasks, preserved responsibility and exact assignment history |
| `/api/hale/v1/applications/{id}/dna/organization` | Checked declared structure, explicit position groups, contracts and separate ownership map |
| `/api/hale/v1/applications/{id}/dna/definitions` | Application-injected workflow definitions, ordered Steps, leaf specifications and exact child references |
| `/api/hale/v1/applications/{id}/dna/knowledge/nodes` | Visible native Knowledge nodes, preserving exact signed revisions |
| `/api/hale/v1/applications/{id}/dna/knowledge/edges` | Stored edges incident to an exact visible node |
| `/api/hale/v1/applications/{id}/dna/knowledge/bindings` | Stored bindings for an exact visible node and optional target |
| `/api/hale/v1/applications/{id}/dna/knowledge/dependents` | The same stored bindings, with other dependency coverage explicitly unavailable |

Practices, reviews, Tasks, organization and definitions accept `id`, `limit`, `offset`
and `snapshot`. Knowledge uses cursor pagination as described below. Pass opaque ids through
URL query encoding, for example `curl --get --data-urlencode 'id=org/reviews/one'`.
The default page is 25 items, maximum 100. Later pages require the preceding
collection snapshot; 409 means restart pagination. Errors have structured JSON
codes and HTTP status. A missing/corrupt Record is unavailable, not an empty
successful catalog. This first implementation bounds snapshots to 10,000 rows
and 16 MiB; exceeding the bound returns an explicit 503.

Each response states its local Record identity, head and revision. Reads do not
fetch a remote or claim remote freshness. A settled approval does not imply
practice activation. An unprocessed practice request is not listed as a practice
until the organization creates its actual document/proposal.

## Organization source

Organization reads require a current Hale compiler on `PATH`, or an absolute
`HALE_BIN` path supplied to the API. The reader checks `dna/org` from an owned
temporary snapshot of committed `HEAD`. It does not build or execute the
organization, check out source in the operator's project, or change its Record.
Uncommitted organization edits are excluded. Available dependency bytes have a
separate fingerprint; an ignored local vendor directory is not covered by the
source commit alone. Missing or unsupported dependencies return an explicit
unavailable-source error instead of an empty organization. Replace the compiler
by restarting the API.

A committed `vendor` tree is inspected from that commit. Otherwise the reader
captures the project's existing local `vendor` directory, without fetching or
upgrading it. It fingerprints all captured dependency files, including unused
ones, so a vendor change can invalidate more than one seed. Hale's native input
resolver must place every consumed input inside the captured source and
dependencies. Symlinks, submodules, special files, unsupported path characters
and imports outside the snapshot are unavailable in this inspection profile.
Captured files must match their committed Git blob identities; archive attributes
that omit or substitute source bytes produce an unavailable-source error.

Committed source is bounded to 16,384 files, 16 MiB per file and 128 MiB total.
Dependencies are bounded to 8,192 files, 8 MiB per file and 64 MiB total. Each
inspection subprocess has a 30-second deadline and its captured output is limited
to 2 MiB per stream. The topology artifact is limited to 8 MiB and the ownership
map to 64 KiB. These are inspection bounds, not a sandbox for untrusted projects.

The response's `basis` names the source commit, dependency origin/digest,
compiler artifact digest and schema, and static coverage. Its page snapshot binds
that basis and the Record head. Static instances and explicit `groups.positions`
membership establish declared structure. They do not establish runtime liveness,
occupancy, effective grants or authority. Source-declared ownership paths remain
separate from compiler instance paths until the application provides that join.
Capacity and retry values travel as decimal strings (or null when unspecified).

## Workflow definitions

The standalone executable advertises `reads.definitions=false` and returns
503 `definitions_unsupported`. It cannot discover an arbitrary application's
in-memory `WorkflowCatalog` from a project directory. A composed head supplies
the application-owned catalog to `ops::DeclaredWorkflowCatalog`, alongside the
same declared `AdmissionLimits` and the loaded-source `DefinitionProvenance`,
then passes that provider to `api::serve(root, port, web, provider)` or `Api`.
The provider must share the actual catalog used by that application; no parallel
sample or reconstructed catalog establishes this capability.

`WorkflowCatalogProvider.supported()` declares support without encoding the
catalog. `snapshot()` captures its native encoded document, digest, limits and
provenance together once for each definitions request. `DeclaredWorkflowCatalog`
implements both methods. A host that replaces its catalog must serialize
replacement against snapshot capture. The query validates a disposable captured
copy, never expands or admits workflows, and preserves the provider's definitions
and any already-bound execution. `reads.definitions=true` reports support; it
does not promise a currently available or valid snapshot.

A successful collection contains every declared revision as a separate item.
Its `id` is an opaque exact-revision identity for URL query use. Ordered `steps`
contain members tagged `leaf` or `child`: leaf specifications preserve the native
Work-request content, while children reference an exact definition revision.
`dependents` reports reverse child references in the captured catalog. No request
falls back to the latest revision. Native revisions and cost ceilings are signed
decimal strings; step indices, attempts and admission limits are unsigned decimal
strings. Native definition reads can preserve zero or negative revisions without
claiming such a definition can be admitted. Knowledge bindings remain opaque text.
These declarations do not establish running Tasks, admitted Works, live state,
occupancy, authority, or mutation support.

`data.basis` identifies the captured catalog digest and format, declared admission
limits, and `trusted_host_claims` about its loaded source revision/module and
optional dependency digest. Empty dependency digest means no dependency claim.
These claims are supplied by the host for the loaded catalog; a fresh checkout
HEAD is not substituted. Pagination binds the entire basis and the Record head;
a catalog, limits, provenance or Record change returns 409 `snapshot_changed`.
A supported empty catalog is a successful empty collection. Unsupported,
unavailable, malformed and semantically invalid catalogs produce structured 503
errors; they never masquerade as empty catalogs. Reader resource bounds produce
`definition_read_limit`, which does not declare the application's policy invalid.

## Knowledge reads

Set `HALE_DNA_MEMORY_DSN_HEAD` to the record's head DSN (`hale dna memory
migrate` prints it). The API reads the graph from memory in its own process,
as the head's role, which may read the graph but never write it: there is no
Knowledge service, URL or read key. The API holds one memory handle, opened on
the first read and kept for the life of the process; a session that no longer
answers is dialled again. It does not forward browser authorization headers.
Without the DSN, `reads.knowledge=false` and reads answer `knowledge_unsupported`.
Configured failures remain unavailable, never an empty successful graph. Support
is a configuration claim, not a health check.

A read answers from the spine's projection as it stands. The head never
projects: a projection that has not reached the record's head answers 503
`knowledge_projection_unavailable` until the body's tick applies it, and a
memory scoped to another record answers `knowledge_scope_mismatch`.

The API derives the reader from its trusted local configuration or existing OIDC
session before any memory read. Public queries accept only `id`, `target`,
`limit` (default 25, range 1..100), `cursor` and `snapshot`. Edges, bindings and
dependents require an exact node `id`; nodes allow optional exact lookup. Unknown
or duplicate fields, noncanonical limits and cursor without snapshot fail before
memory is read. A missing or hidden exact node receives the same 404. A target is a
native locus-path relevance context, not identity or authority.

Knowledge pages use `returned`, `has_more` and `next_cursor`, with no offset or
total. Continue with the same kind, id, target, limit and snapshot. The snapshot
hash binds the Record, projection watermark and graph generation, routed receipt
visibility, reader scope and target. All graph collections for that captured view
share it. A changed view returns 409. The public adapter strictly validates every
nested response, exact Int64 decimal string and the canonical basis hash; it
rejects a wrong Record, reader scope, target or inconsistent page. It rebuilds
public errors with known codes and generic messages, without the store's
diagnostic text or credentials.

Node `projection_state` is the native projection lifecycle; `source_provenance`
remains null. Historical empty names remain unknown; empty `supersedes` can also
mean that the predecessor is protected and unavailable in this view. Its absence
does not prove there is no predecessor. Edges and bindings
are actual stored relationships. Coverage is `bindings=complete` and
`runs/definitions/practices=unavailable`; this slice does not establish runtime
consumption, source-provenance backfill, curation or write authority.

`Api.knowledge` accepts the structural `KnowledgeProvider` interface for a
composed host or focused tests. `supported()` does no read; `read(request)` gets
the captured Source and authenticated reader. The existing `serve` signature
constructs `LocalKnowledge` (`dna/api/knowledge_memory.hl`) internally. The
read keeps its own bounds: a request of at most 16 KiB, a Record of at most
10,000 rows and 8 MiB, at most 10,000 stored rows scanned per page, a cursor of
at most 8 KiB and a response of at most 1 MiB; past any of them the read is
`knowledge_read_limit`. The public API is source-built; the compiler's embedded
DNA inventory does not currently package this API seed. Packaging a
distributable API belongs to the service deployment work.

## Knowledge commands

Knowledge commands (`dna.knowledge.edge.link@1` and `.unlink@1`, and the
reviewed node and binding profiles) are admitted into the Record in the API's
own process, by the operations' `KnowledgeCommands` (`LocalKnowledgeCommands`
in `dna/api/knowledge_memory.hl`). There is no command key and no service to
reach. Set `HALE_DNA_KNOWLEDGE_COMMAND_POLICY` to the path of an explicit
authority document; without it the API advertises no Knowledge command and
answers `commands_unsupported`. The API reads the document once, when it
starts, so an edit to the file does not change the basis a running API decides
on. A document that is empty, larger than 64 KiB or invalid for this Record
stops the API at startup (exit 2).

```json
{
  "format": "dna.knowledge-authority/1",
  "application_id": "<Record genesis identity>",
  "grants": [
    {
      "mode": "local",
      "name": "alice",
      "authority": "knowledge-editor",
      "edge_link": "direct",
      "edge_unlink": "direct",
      "recover": true
    }
  ]
}
```

`ops::KnowledgePolicyCodec` (`dna/operations/knowledge_policy.hl`) decodes it
against the Record's identity and its `dna.trust` (`local` or `signed`; a
signed Record signs the facts the commands admit). Grants match the exact
authenticated mode/name pair, at most 128 of them. `edge_link` and optional
`edge_unlink` are `direct`, `review` or `deny`; omitting `edge_unlink` denies
removal. The optional `node_propose`, `node_revise`, `node_retire`,
`binding_bind` and `binding_unbind` are `review` or `deny`, and a `review` grant
among them needs its `node_scopes` or `binding_scopes` (`{author, target}`
pairs, at most 32, never lateral). The public contract is in
[contract/v1](contract/v1/README.md#optional-knowledge-relationship-command).

## Commands

The record's commands are gated topics on the head's api binding, not
HTTP routes: one Unix socket per record,
`$XDG_RUNTIME_DIR/hale/dna/<record id>.sock`, else `<root>/.hale/dna/<id>.sock`
(`LOTUS_API` overrides). `hale describe <socket>` lists a caller's
slice — the calls it may use; `hale check --dump-api dna/api` prints the
full description, which is the contract for these calls (`contract/v1`
remains the contract for the HTTP reads and the knowledge commands).

| call name | subject | payload type (fields) | gate |
|---|---|---|---|
| `PracticePropose` | `dna.commands.practice.propose` | `PracticeProposal { request_id, subject_digest, text, rationale }` | `position` |
| `ReviewVerdict` | `dna.commands.review.verdict` | `ReviewDecision { request_id, review_id, subject_digest, verdict, comment }` | `reviewer` |
| `OrganizationPropose` | `dna.commands.organization.propose` | `OrganizationProposal { request_id, source_head, module_digest, dependency_source, dependency_digest, record_head, source_text, rationale }` | `owner` |
| `TaskCreate` | `dna.commands.task.create` | `TaskCreation { request_id, record_head, outcome, to }` | any authenticated peer |
| `TaskReassign` | `dna.commands.task.reassign` | `TaskReassignment { request_id, task_id, assignment_digest, assignee, to }` | `owner` |
| `PersonRetire` | `dna.commands.person.retire` | `PersonRetirement { request_id, person, subject_digest, to }` | `owner` |
| `AttemptClaim` | `dna.commands.attempt.claim` | `Claim { request_id, record_head, performer_kind, performer, capabilities, data_classes, organizations, ttl: Int }` (the three lists are space-separated words) | `position` |
| `AttemptOutcome` | `dna.commands.attempt.outcome` | `Outcome { request_id, attempt_id, holder, token: Int, disposition, result, result_ref, narrative, evidence, receipts, hat_digest, hat_head, hat_watermark: Int = -1, prompt_digest, renderer }` (evidence/receipts are JSON arrays as text) | `position` |
| `AttemptRenew` | `dna.commands.attempt.renew` | `Renewal { request_id, attempt_id, holder, token: Int, ttl: Int }` | `position` |
| `AttemptRelease` | `dna.commands.attempt.release` | `Release { request_id, attempt_id, holder, token: Int, why }` | `position` |
| `FrictionFile` | `dna.commands.friction.file` | `Friction { request_id, record_head, position, attempt_id, text }` | `position` |
| `CommandLookup` | `dna.commands.lookup` | `Lookup { request_id }` | any authenticated peer |

The caller is the socket peer: `dna.unix.member` maps its uid to a
person, and rows record `local/<person>`. `TaskCreate` and
`CommandLookup` need no role — any authenticated peer may call them;
the other calls are outside a caller's slice (`unknown`) unless the
mapped person holds the named role. Every reply is a `CommandReply { ok, code, application_id, head,
revision, receipt }`, its receipt exactly what an HTTP command receipt
used to carry. `CommandLookup { request_id }` is recovery, in place of
the old `GET /commands?request_id=`. The HTTP head answers 405 to every
mutation now, except a knowledge command or a draft POST.
`/capabilities` no longer carries `writes` or any command profile; it
carries `api{transport,socket}` instead.

`hale dna work`, the legs client, is being switched from the HTTP route
to this socket by the DNA line; the socket is its target.

## Practice and Review command providers

`PracticePropose` and `ReviewVerdict` ([Commands](#commands)) are gated
`position` and `reviewer`. `api::Commands` composes `NoCommands` by
default, which supports neither: an application binds its own native
`CommandProvider`, using the same startup and authentication path. This
does not install a durable command implementation: the application owns
it. Bind the provider to a named locus in the hosting scope and keep it
alive for the head's lifetime.

The provider exposes operation/version-aware `supported`, typed
`submit(context, CommandRequest)` and shared `lookup(context, request_id)`.
Supported operations occupy the same application/principal/request namespace; reusing
a key with different operation/content must conflict. Trusted context carries
the resolved principal — the socket peer, mapped through `dna.unix.member` —
and the application's Record binding.
The gate validates closed `CommandRequest` values, byte limits,
expected principal and exact application/subject consistency. The
expected principal is a precondition only; it cannot set the authenticated
actor. An identity change returns `command_context_changed` before dispatch.
Unknown fields, caller-supplied authority, duplicate keys and invalid Unicode
are rejected. No CLI command, domain history repair or generic bus publication
occurs in this adapter.

`CommandLookup { request_id }` is the recovery call: it still requires
current receipt access and resolves exactly one request.

The provider owns current authorization, canonical request equivalence, durable
principal/application/key scoping, exact-head admission and interrupted proposal
and verdict progression. It must resolve an existing request before applying fresh-subject
preconditions to a retry. Unrelated Record movement is not a lock;
the provider checks the pinned subject and performs its internal admission CAS.
Receipt lookup joins the exact native request, not the latest practice metadata.

Typed provider results include their captured source basis. The gate checks
scope, identity and receipt-state consistency before replying, and uses
that basis rather than the pre-submit Record head. A recorded reply means a recorded or
unresolved command state, never Review approval or practice adoption. A successful
proposal proves candidate and Review creation. A successful verdict command proves
that this specific verdict was accepted. Review settlement and activation remain
separate fields in both cases. `hale check --dump-api dna/api` is the
contract for the call's request and reply shapes.

`dna.review.verdict@1` targets an exact pending practice-candidate Review with
separate candidate-digest, expected-principal and pending-state preconditions.
Its literal comment may be empty and is bounded to 2048 UTF-8 bytes; supported
choices are approve, reject and revise. The provider derives reviewer authority
from trusted policy, checks eligibility and current authority at the delayed
decision, and must not reinterpret an already-settled approval as permission to
retry application. Mutation Reviews, reapproval and abstention are excluded.
The first native practice-review profile is non-mutation, has no approver quorum
and declares required authority `board`; that label cannot establish the caller's
grant. Review reads expose existing `is_mutation` and `approvers` facts so clients
can distinguish this shape; those fields are optional for older read responses.
An overall settled Review cannot establish this caller's command outcome:
the provider must correlate the exact request, including refusal beside another
command's approval. Existing Review-ID-only native joins do not establish that
durable correlation. Unsupported providers advertise no verdict capability.

`tests/commands_api_test.hl` and `tests/commands/main.hl` exercise the
handlers with scripted providers, in-process. Neither appends native
command facts nor provides restart durability; neither is proof that the
administration loop is complete.

## Handed Tasks and reassignment

`GET /api/hale/v1/applications/{id}/dna/tasks` exposes native human-handoff
history through profile `dna.task-administration.v1`. It is distinct from wf1
workflow execution. Rows retain the outcome, current state and assignee,
obligation, explicit acceptance binding, evidence requirements/reference,
waiting reason and ordered handoff/reassignment history. Acceptance and evidence
references are opaque strings. `assignment_digest` binds the relevant native
lifecycle, including intervening and terminal events; `reassignment_supported`
states factual support, not the caller's authority. Each row also carries
`usage`: the model calls attributed to the Task — its plan, its Mutations'
attempts and their verdicts — as the same object the executions carry.

Reads require a trusted-local principal and a complete Record-only projection
before Ledger adoption. They serve only the current captured head: an old
`snapshot` returns 409, rather than historical Task contents. Hidden Tasks are
omitted, including identifiers and counts; incomplete, unsupported or over-limit
history is unavailable. An unavailable projection is not an empty Task list.

Task reads alone also accept `assignee=<exact person identity>`. It is decoded
once, must be 1..256 UTF-8 bytes without C0/DEL, and cannot be empty or repeated.
The filter uses the current recorded assignee, including terminal Tasks with a
prior handoff, before pagination and total counts. Filtered data echoes
`assignee`; unfiltered data omits it. With `id`, both identities must match or
return 404. A later page still requires `snapshot`; other read routes reject
`assignee`. This does not establish position ownership, complete personal
responsibility or permission to reassign.

The [native command provider](practice_review/README.md) can additionally enable
`dna.task.reassign@1` with `HALE_DNA_TASK_POLICY`. Its existing
`HALE_DNA_COMMAND_POLICY` remains required; Task authority is independently
configured. For example, this is the complete closed Task policy shape; replace
the application ID and exact local names with your own:

```json
{
  "format": "dna.task-authority/1",
  "application_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "owner": "operations",
  "members": ["mara", "dev"],
  "grants": [
    {"mode": "local", "name": "riley", "reassign": true, "recover": true}
  ]
}
```

The policy is immutable startup configuration, bound to the application's Record
genesis ID and identified by its document digest. It permits 1–64 unique members
and up to 64 unique local principal grants; unknown fields, invalid configuration
or another application's policy refuse startup. Missing configuration grants
nothing. `owner` is an explicit authority label, not a source-position or Task-ID
mapping. Member eligibility also requires current visibility and no recorded
retirement. Both the former and new assignee must be eligible; an unassigned
legacy handoff cannot use this reassignment operation. The authenticated actor
needs the exact grant, not membership in the assignment group. `USER` supplies
no grant. Restart the provider to load policy changes; Record admission does not
atomically fence a mutable external policy file.

With a matching source-built composed binary, run it directly with the
policies in its environment:

```sh
HALE_DNA_COMMAND_POLICY=/absolute/path/authority.json \
HALE_DNA_TASK_POLICY=/absolute/path/task-authority.json \
  dna/api/practice_review/practice_review /absolute/path/project 8793
```

Under the launcher the [head](#head-project-service) starts this binary itself
as the attached project's API child, with the two policies it synthesized
under `<root>/.hale/dna/face/` when the operator wrote none:

```sh
dna/face/start.sh /absolute/path/project --api /absolute/path/practice_review --port 8792
```

`TaskReassign` ([Commands](#commands)) is gated `owner`; `/capabilities`
carries no profile for it. Read access or a selected working locus
does not grant this operation. Its payload is `TaskReassignment {
request_id, task_id, assignment_digest, assignee, to }`.

Use the exact returned Task ID, assignment digest and current assignee. The
service checks these and current eligibility in the captured Record, then admits
one `task.reassigned` fact at the exact predecessor. This fact is both the
durable command identity and the assignment effect. The same Task remains open;
its obligation, acceptance, evidence and prior assignment history are preserved.
No Body, Host or Knowledge memory is needed for this direct operation.
The whole request is bounded to 32768 encoded bytes, request keys to 128 bytes
and person/Task identities to 256 bytes.

The receipt's `task` fields distinguish `applied` from `unknown`, preserving
exact `from`, `to` and native `event_id`. A recorded reply alone proves no effect. Fresh
Task readback is separate from the recorded receipt and may show later changes.
The application/principal/request key shares the existing Practice/Review/Organization
namespace; different operation or content conflicts. Recover only through
`CommandLookup { request_id }`, with the same application prefix and current
receipt grant. `recover` is independent of `reassign`, so an operator may retain
lookup after new writes are denied. Lookup does not repeat the mutation and
resolves the original receipt even after the Task changes.

This profile does not retire people, transfer responsibility across owners,
modify source ownership, complete Tasks or control workflow execution. OIDC,
signed trust, Ledger-backed administration and source-to-live ownership joins
need their own supported service contract. Existing CLI Task commands are not
made part of this provider by this call.

## Head: project service

`dna/api/project_service` is the head (GH #965): the one process the
browser talks to. It serves the shell, owns the operator-machine state — a
project registry, a receipt journal, run and child files under
`${HALE_DNA_HEAD_STATE:-${XDG_STATE_HOME:-$HOME/.local/state}/hale/dna/head}` —
and reverse-proxies every `/api/hale/v1/applications…` request to the attached
project's API child (`practice_review <root> <api-port>`) on the loopback, so
the browser has one origin and the child's exact-`Origin` guard holds untouched.
Switching a project replaces the child. The head re-implements no verb: each
operation runs `$HALE_BIN dna …` (or `git config` for the two forge keys) as a
detached child under `timeout -k 10 <secs> sh -e`, with its pid, exit and log
as files, so restarting the head interrupts nothing and the next head re-adopts
the children whose command line matches what it recorded, or is still one of
the head's own wrappers on its way there.

```sh
hale build dna/api/practice_review
hale build dna/api/project_service
HALE_BIN="$(command -v hale)" \
  ./dna/api/project_service/project_service 8792 dna/face/web dna/api/practice_review/practice_review 8793 [/absolute/path/project]
```

Four routes, described in `contract/v1` beside the Record routes, all under the
head envelope `{"api_version","head":{"profile":"dna.head.v1","principal","active"},"data"}`:

- `GET /api/hale/v1/head` — detached or attached, the active project and its
  API child, the body child, credentials **by name** (what the
  catalog needs, which file sources exist, which names are set), the running
  receipt (`busy`), and every operation with its availability.
- `GET /api/hale/v1/head/projects[?id=…]` — the registry; with `id`, the
  policies' `authority` booleans, connections, handoffs, secret names and the
  project's receipts, projected in-process from the Record.
- `POST /api/hale/v1/head/commands` (the same `Origin` / `Content-Type` /
  `X-Hale-Command: 1` guard as the Record commands) and `GET …?request_id=` —
  one closed request `{request_id, operation, operation_version:"1", context:{head:"local",
  application_id}, target, preconditions:{principal}, arguments}`, one receipt
  `recorded → admitted|refused → running → succeeded|failed|outcome_unknown`.
  Identity is `command-<digest>` over the principal and `request_id`; an
  identical retry replays the stored receipt (never re-executed), a changed
  request under the same id is 409 `request_conflict`. Runs settle on the next
  request from their files: exit 0 succeeded, non-zero failed, 124 or a passed
  deadline `timed_out` — `outcome_unknown` when the verb is external (a push,
  a sync, a probe), never a fabricated failure. Row-writing operations carry
  `outcome.record = {head_before, head_after, rows}`.
- `GET /api/hale/v1/head/logs?run=<command_id>|child=api|body[&offset=N]` —
  64 KiB pages of a run's or a child's log.
- `GET /api/hale/v1/head/events` (GH #986) — a `text/event-stream` the head
  writes `event: changed` to: `data: rows` when a row landed in the active
  project's organism, `data: head` when the head's own state moved (a receipt
  journaled, a run or a child started or ended, the API child answering). An
  event carries nothing to render; the face reads the API again. A comment
  line every 15 s keeps a quiet stream open; at most 16 streams, one more is
  503 `busy`. The rows come from the nerves: every node publishes
  `head.row.landed` for each row its view gains, and the head subscribes to the
  active project's organization — and every owner's — as the head's user, on
  `HALE_DNA_NATS_URL_HEAD`, else on the server of `HALE_DNA_NATS_URL_OWNER`
  (which it hands a body it starts), else on the project's compose `nerves`
  while `hale dna dev` has it up; with none, only the head's own state is
  pushed.

Operations (all version 1): `dna.project.create` / `init` (600 s runs of
`hale dna new` / `init`, then an attach), `attach` / `detach` / `forget`
(inline; attach validates the worktree, the Record, the `dna/` seed, and
refuses `principal_unsupported` / `trust_unsupported` for OIDC or signed
projects), `dna.project.sync` / `publish` (a commit and push of the genome,
then a sync), `dna.forge.configure` / `sync`, and the fourteen body, secret,
model, connection and handoff operations of the head's contract.
A secret is never a value on this wire: `dna.secret.set` names a **source** —
a 0600 one-line file under `${XDG_CONFIG_HOME:-$HOME/.config}/hale-dna/sources/`
or an environment variable the run's shell reads — and the file is unlinked
once the verb succeeded. A request carrying `value` anywhere is 400.

The head is trusted-local: its principal is `USER`, every child runs as it,
and it proxies no `/auth/*`. Restarting a head over the same state directory
answers every earlier `request_id` with the same terminal receipt and
re-attaches the last activated project. `tests/journal_test.hl`,
`tests/operations_test.hl` and `tests/head_api_test.hl` run under
`HALE_HEAD_BIN` and `HALE_API_BIN`; each makes its own scratch root.
## Raising work

`dna.task.create@1` is the face's `hale dna ask`: it records the same
`intent.requested` row the CLI writes, so the host beside the organism relays it
and the organism admits it exactly as it admits a CLI ask. The row's entity is
the intent id (`i` plus the lower-case hex of the current monotonic
milliseconds, as the CLI mints it), its author is the acting principal, and its
body is one flat JSON object whose first three keys are `outcome`, `from` (the
principal) and `to`, followed by the command fields the other operations carry
(`command_format: dna.task-create-command/1`, `command_id`, `command_payload`,
`command_fingerprint`, `command_authority`, `command_authority_basis`,
`command_record_head`). It carries no `via` and no `intent_id`: this head
publishes nothing on the nerves, and a node's relay splices the id in before
the last brace when it publishes.

Every [native command provider](practice_review/README.md) supports it; there is no
policy grant. The authority is the authenticated principal, as with the CLI,
and whether `to` is this organization's to admit is the organism's judgment,
recorded as an `intent.refused` row. `TaskCreate` ([Commands](#commands))
is open to any authenticated peer; `/capabilities` carries no profile for
it, and it contributes nothing to `read_only`. Its payload is
`TaskCreation { request_id, record_head, outcome, to }`.

The target is the Record itself: the Task is minted by the organism after the
ask is admitted, so the request can name only the application. `record_head`
follows the `dna.organization.propose` precedent: the row lands at exactly the
head the request was prepared against, or the request is refused with
`stale_subject` and must be prepared again against the current head.
`outcome` is 1..8192 UTF-8 bytes with control bytes preserved; `to` is a
position, 1..256 bytes. The whole request is bounded to 32768 encoded bytes.

The receipt is `succeeded` once the row is appended (the ask is admitted; the
organism's answer is a separate fact) or `outcome_unknown` when the append is
uncertain. `subject_digest` is the Record head the request was prepared
against. Its `task_create` object names the minted `intent_id`, the row's
`event_id`, and re-derives the organism's answer from the Record on every read:
`intent_state` is `requested` until the organism answers, then `offered`,
`refused`, or `born` with `task_id` filled from the `task.born` row whose body
starts with `<intent_id>: `. A born-but-unhanded Task is absent from
`/dna/tasks`, so `CommandLookup { request_id }` is how the face follows the
ask; nothing else is re-read. After `ledger.adopted` new asks are refused
(`commands_unsupported`) like every other operation here; recorded asks stay
recoverable.

Two asks minted in the same millisecond on one host would share an id and the
organism admits one Task per id; this head steps an id its Record already holds
to the next millisecond, which the CLI does not. A CLI ask beside a face ask
carries no command fields and is invisible to command lookup.

## The hat

`GET /api/hale/v1/applications/{application_id}/dna/context?id=<work>` answers one
Work's context as structure (GH #946): the position's identity and charter, the
practices ratified for the Work's target resolved to text with their ids, the
knowledge bindings the owner named, the tool grant, the output contract, the data
class, the Work's history as facts, the Record head and memory's projection
watermark it was rendered at, and a `digest` over all of it. It is never a
prompt: rendering is a leg's, and the renderer's version is evidence of its own.

The read is trusted-local, like the executions it is drawn from, and `id` is
required. The position is the graph's `position:<name>` id (GH #1085): the
performer kind of the attempt admitted last, or the kind the request selects
before one is (`edit` is `position:editor`; a judgment is `position:agent`; a
person's work `position:human`); its `charter` is the record's `graph.node`
text for that id, `""` until the graph names it. The practices come from memory
under the head's role (`HALE_DNA_MEMORY_DSN_HEAD`), as structure — `id`, `name`,
`text`, `kind`, `author` — from one snapshot, the watermark read in the same
transaction as the ranked bound, and the hat is kept in memory by its digest
(`hats`, insert if absent); `practices_status` says
`resolved`, `no memory` or `unavailable`, and `watermark` is `-1` when none was
read. `digest` is sha256 over the canonical body with the digest itself left
out: rendered twice at one head it is one digest, and a row that moves the head
moves it. An attempt records the hat digest and the head and watermark it was
rendered at; replay renders from the recorded hat, never the live graph.

## Attempts: a leg's claim and outcome

Two gated topics ([Commands](#commands)) are the spine's whole API to a
leg (GH #946): `AttemptClaim` and `AttemptOutcome`, both gated `position`.
A leg holds nothing between tasks and has no database role: the head
takes the claim in memory for it and writes the rows; the owner still
admits and settles.

`AttemptClaim`'s `Claim` payload targets `dna.work` with the
application's id, names the Record head the request was prepared
against (`record_head`; the claim is not
fenced on it — memory's conditional insert is the race, and a leg a row behind
gets a receipt, never `stale_subject`), and carries a filter:
`performer_kind`, `performer` (the leg's identity: `position:<name>`, with `#<n>`
for one worker of several — a position the graph names or one of the
organization's own, else refused), `capabilities` (words the
leg has; a Work's `requires` must all be among them), `data_classes` (classes it
may see; none is no class, and nothing is handed over), `organizations` (owners it works for on a shared Record,
`-` for the sole owner; none is any) and `ttl` (1..86400 seconds). The head picks
the first admitted, outstanding attempt of that kind that fits — asked to run,
no outcome, not held by another leg under a live lease — takes memory's claim
`attempt:<id>` for `performer` with the TTL under the head's role (the store
decides between two legs racing; without memory nothing races and the claim is
granted), and appends `attempt.claimed` naming the holder, its token, until
when, and the principal that took it. The receipt's `attempt` object is the lease as a value: `state: claimed`,
`attempt_id`, `work_id`, `task_id`, `performer_kind`, `holder`, `token`, `until`,
`event_id`. Nothing fitting, or everything held, is `state: refused` with the
`reason` — which names the class or the owner that stood in the way — and no row.

`AttemptOutcome`'s `Outcome` payload targets `dna.attempt` with the attempt id, under the lease
(`holder`, `token`; `subject_digest` is `lease:<holder>@<token>`),
and carries the outcome: `disposition` (`done`, `failed`,
`declined`, `timeout`), `result` (at most 16384 bytes) or `result_ref`,
`narrative`, `evidence` (the calls the leg made, as `model.called` evidence
objects: adapter, backend, models, digests, tokens, cost), `receipts`
(`{class, body}`: a `customer` or `confidential` body goes to memory alone
through `receipt_file`, the rest are git receipts) and, when the leg wore one,
the hat — `hat_digest`, `hat_head`, `hat_watermark`, `prompt_digest`, `renderer`,
the five together or none. The head refuses — a
receipt with the reason, no row — an attempt never admitted or never asked to
run, one already settled or already handed back (`duplicate`: one outcome under
one lease), a lease that is not this holder's at this token now, in the record
or in memory (`stale`), and an outcome from a principal other than the one that
claimed. Otherwise it files the
receipts and appends `attempt.outcome_requested`; the receipt is `requested`. A
node relays the row onto the nerves (`dna.work.submit`); the owner journals the
calls on the attempt (tokens per task hold out of process), settles it as it
settles every reply (`attempt.outcome`, naming the request and carrying
`result_ref` and the hat) or refuses it with
`attempt.outcome_refused` naming why and the request; `CommandLookup { request_id }`
re-derives `settled` with the disposition, or `refused` with the reason and the
refusing row's id. Once the organism has adopted the ledger, both calls and
the hat read and write it under the head's role (`HALE_DNA_MEMORY_DSN_HEAD`);
without it they answer `commands_unsupported` / `context_source_unavailable`.

Three more gated topics carry a leg's loop (GH #946 slice 4), all gated
`position`: `AttemptRenew` (its `Renewal` payload targets `dna.attempt`, the
lease, `ttl` 1..86400; the lease extended and the token kept, `state:
renewed`), `AttemptRelease` (its `Release` payload carries the lease and
`why`; `attempt.released`, `state: released`, and the attempt is
another leg's to claim) and `FrictionFile` (its `Friction` payload targets `dna.friction` with the
application's id, `record_head`, `position`
(`position:<name>`), `attempt_id` (or `""`) and `text`; `friction.filed` on the
attempt, else on the position, `state: filed`; nobody admits it). A lease that is
not this principal's, this holder's at this token now is refused as for an
outcome. `hale dna work` targets this socket ([Legs](../../docs/src/dna/legs.md)).

`/capabilities` no longer carries a profile for these five calls; `hale
describe <socket>` lists a caller's gated slice. `reads.context` says the
hat is readable.

## Identity and content

With no configured principal source, or `dna.principal=local`, loopback access is
trusted and the reader is server-derived. `dna.principal=oidc` reuses the existing
issuer, client, redirect and member mapping configuration. Start at `/auth/login`;
`dna.oidc.redirect` must point to this service's `/auth/callback`. The service uses
`HALE_DNA_OIDC_SECRET` for the configured exchange. Missing, unmapped or expired
sessions cannot read the API. Unknown principal modes fail startup. No query
parameter changes the authenticated member or supplies an acting role.

Sessions are in memory. Restart requires another login; Record identities and
read results persist. The existing UI's legacy command routes are never routed
through this server. A mapped member has the existing Record-reading scope;
position-scoped permissions and remote CLI tokens are future work.

The face's optional shell and assets are public static content with no Record
data. They can show the sign-in state before authentication. Record reads still require a valid session in OIDC mode. After
successful sign-in the existing callback redirects to `/`, where the face
loads the authenticated API data.

Practice documents must match their content digest, proposal metadata and a
supported native canonical encoding. Redacted/protected text is suppressed even
when a stale local blob remains. After any Ledger adoption the API still serves
Record metadata, but withholds canonical text and derived Review questions until
a later adapter can establish current Ledger receipt visibility. Typed decision
outcomes remain distinct from withheld display text. The legacy local CLI keeps
its existing receipt-rendering behavior.
Ledger abandonment does not restore visibility: it does not copy historical
receipt restrictions back into Record. Unknown classification metadata and
conflicting Review subject references likewise cannot establish readable text.

See the [contract](contract/v1/README.md) for response schemas and the
[query layer](../operations/README.md) for projection semantics.

## Validate

From the source checkout, run the native Hale contract and integration tests:

```sh
export HALE_API_CONTRACT_ROOT="$PWD/dna/api/contract/v1"
hale test dna/api/contract/v1/tests
hale check dna/api
hale build dna/api
HALE_BIN="$(command -v hale)" HALE_API_BIN="$PWD/dna/api/api" hale test dna/api/tests
hale check dna/api/practice_review && hale build dna/api/practice_review
hale check dna/api/project_service && hale build dna/api/project_service
HALE_BIN="$(command -v hale)" HALE_HEAD_BIN="$PWD/dna/api/project_service/project_service" HALE_API_BIN="$PWD/dna/api/api" hale test dna/api/project_service/tests
hale test dna/operations/tests
cargo test -p hale-dna -p hale-iris
```

The integration suite starts only owned loopback processes and temporary Git
repositories. It validates live responses against the same contract as the
fixtures, including a local OIDC issuer, restart, pagination, refusal
and content suppression. It requires socket access; no paid model, production
database or organization body is involved. `HALE_API_BIN` selects an already
built service. CI builds with the checkout's compiler and runs this suite.
The native contract checker supports the schema profile used by this API and
rejects unsupported schema constructs; it is not a general JSON Schema validator.
`hale check --dump-api dna/api` prints the socket's full description — the
contract for its gated commands ([Commands](#commands)); `contract/v1`
stays the contract for the HTTP reads and the knowledge commands.

### Organization source preparation

The same host opt-in also exposes `dna.organization.ownership.draft.v1` through
`GET` and `POST /api/hale/v1/applications/{application_id}/dna/organization/ownership/draft`.
This captures the committed `dna/org/owners` map, including an absent/empty map,
and previews changes through the existing native `Ownership` model and
`owners_affected`. The response includes candidate assignments, memberships,
hosting, inherited owners for every scope named in either map, and affected
owner review members (candidate membership, falling back to original membership
as the domain does). Empty memberships and a change to single-owner mode are
reported; they do not establish permission to adopt the source.

The ownership editing profile is bounded to 16 KiB and 256 entries. Each
non-comment line must be `scope = owner`, `owner: members`, or `host = owner`;
names cannot contain whitespace or assignment/list/comment delimiters. Empty
owner/member lists are representable. Duplicate normalized scopes, duplicate
memberships and repeated hosts are refused instead of silently accepting the
domain parser's first/last-match behavior. Existing maps outside this profile
remain readable through the Organization read API but are not editable here.
The endpoint uses the same exact principal/source/dependency/Record fences,
Origin/framing guard and 32 KiB request bound as organization source drafts.
It performs no project or Record writes. Live obligations, instance bindings,
publication and activation remain unavailable in this preparation profile.

Set `HALE_DNA_ORG_DRAFTS=1` on the native API host to offer the optional
`dna.organization.draft.v1` capability. The browser can then read and edit the
exact committed `dna/org/main.hl`, inspect a source diff, validate the complete
captured organization with Hale, and export that exact candidate with its
validation evidence. Neighboring source files and captured dependencies retain
their original bytes. The module limit is 16 KiB; the JSON request limit is
32 KiB; the draft projection limit is 256 static instances.

`GET` and `POST /api/hale/v1/applications/{application_id}/dna/organization/draft`
share the normal authenticated session. POST pins source commit, dependency
identity, module digest, Record head and expected principal, and requires the
configured Origin and command intent/framing headers. Changed basis refuses the
draft. The host checks a disposable snapshot and never edits the working tree,
Record or running application. This is source preparation: publication,
retirement, reassignment of live obligations and activation require the owning
services. Viewing a chart position grants no authority. The capability is off
by default and does not change `read_only` or any command capability.

### Recorded workflow reads

`GET /api/hale/v1/applications/{application_id}/dna/workflows` exposes the merged
native `WorkflowProjection` over recorded workflow facts. `reads.workflows` is
advertised for trusted-local mode. The collection uses the existing `id`, `limit`,
`offset` and `snapshot` query contract. Lists contain visible root summaries;
exact details include the immutable bound nodes, admitted attempt histories and
native transition order. Decimal strings preserve native integer identities.
`recursive_workflows: false` still means the head supplies no execution service.

This initial adapter reads complete Record routing-0 histories only. Any past
Ledger adoption makes it unavailable, including after abandonment; it does not
merge memories or guess a missing Ledger. OIDC reads require an owning-service
visibility provider and are unavailable here. Reference visibility changes or
non-public/non-internal Work classes suppress an entire affected execution.
The response does not dereference receipt bodies, infer runtime identities, or
make a native transition. Reader budgets are 2,048 Record events, 512 unique
workflow facts, 64 KiB per fact, 512 KiB cumulative fact bodies and 256 bound
nodes. Exceeding them reports `workflow_read_limit`, not an invalid workflow.

Every execution and each of its attempts carries `usage` (GH #946): the model
calls summed from their `model.called` rows — `calls`, `input_tokens`,
`output_tokens` and `cost_micros` as unsigned decimal strings — with a
`by_position` breakdown keyed by the graph's position id (`position:<name>`)
and a `by_backend` one keyed `<backend>/<model>`. An
execution's sums cover the Leader's plan of the ask it was born of and every
attempt of every Work under it, child workflows included; an attempt's are its
own. The projection is `dna/operations/usage.hl`, and `hale dna history` prints
the same sums.
# Person retirement

The composed local command provider accepts an optional `retire` boolean alongside
`reassign` and `recover` in each `dna.task-authority/1` grant. Omission preserves
the existing policy and disables retirement. Keep an explicit `retire: false`
with `recover: true` when revoking writes but retaining request recovery.

`GET /api/hale/v1/applications/{application_id}/dna/people?id=<person>` returns
the exact person state, complete supported responsibility plan, plan digest and
eligible successors. An optional `snapshot` pins its Record head. The read
refuses protected or unsupported affected history instead of reporting a partial
plan. `PersonRetire` ([Commands](#commands)), gated `owner`, uses the same
identity namespace as the record's other commands: payload
`PersonRetirement { request_id, person, subject_digest, to }`, targeting
`dna.person`. An empty successor is permitted only for a complete plan with no held Tasks.

Publication creates ordinary Task reassignment facts and a retirement fact
off-ref, then publishes the complete chain with one Record compare-and-swap.
Recovery verifies that exact chain and its transfer manifest. The browser retains
only request identity across reloads and never resubmits an uncertain request.

This profile requires the updated Body and CLI writers from the same deployment;
their exact admission checks prevent concurrent handoffs to retired people.
It supports local Record authority, at most 32 transferred Tasks, and no prior
Ledger adoption. Source memberships and declared positions are separate. Mixed
old writers, cross-owner transfer and distributed Record reconciliation are not
covered by this local atomic publication profile.
