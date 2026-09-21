# DNA read API

The service API reads practices, reviews and handed Tasks from an existing local DNA Record
and inspects the project's committed organization source. An application-composed
head can also expose its real workflow catalog. Configured Knowledge reads use
the private native state service and its authoritative graph store. The API runs
separately from the organization body and Iris. Record and source reads do not
require Postgres; configured Knowledge reads require the state service.
Typed projections in `dna/operations` are shared with the native CLI. The API
does not invoke CLI operational commands or parse terminal rendering. Organization
inspection uses the native compiler's machine-readable topology export.

This is an experimental source-built service. An optional
[Iris cockpit](../../iris/cockpit/README.md) browses these reads from the same
origin. The checkout provides a [cockpit launcher](../../iris/cockpit/README.md#run-locally);
there is no installed `hale dna api` subcommand or Compose service profile yet.
Product scope and remaining service work are tracked in
[#690](https://github.com/hale-lang/hale/issues/690).

## Run

For the browser and the services together, `./iris/cockpit/start.sh [PROJECT]`
builds this checkout's [project service](#head-project-service) (the head) and
its per-project native API (`dna/api/practice_review`) in temporary storage and
serves the eleven browser assets from the head. The project is optional: without it
the head starts detached and the browser's Projects workspace creates,
initializes or attaches one. Use `--api BINARY` for an existing
application-composed API and `--head BINARY` for a built head. The launcher
inherits private service configuration; the head is trusted-local and refuses
a project configured for OIDC. See the cockpit README for fresh-project source
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

To serve the cockpit as well, pass its static directory as the third argument:

```sh
./dna/api/api /absolute/path/to/a/dna-project 8792 "$PWD/iris/cockpit/web"
```

Open <http://127.0.0.1:8792/>. The asset whitelist is `/`, `/app.js`, `/runtime.js`, `/application.js`,
`/definition-draft.js`, `/organization-draft.js`, `/knowledge-draft.js`, `/task-administration.js`, `/projects.js`, `/task-create.js` and `/styles.css`; `/iris/observer.json` supplies static connection metadata.
URLs never become filesystem paths. All eleven assets must exist and be nonempty
at startup. They are loaded once, so restart after changing them. API-only mode
retains its existing routes. No legacy mutation routes are enabled in either
mode. Browser assets and authentication share the API origin; there is no
cross-origin API contract.

Optional `HALE_IRIS_OBSERVER_ORIGIN` supplies one trusted HTTP(S) origin for
the browser's independent native observation. Empty/unset permits only same-origin
connections. A valid configured origin is returned in the public
`/iris/observer.json` profile `hale.iris.observer.v0` and added exactly to CSP
`connect-src`; the API does not fetch or proxy it. Nonempty invalid configuration
refuses shell startup. Paths (including a trailing slash), userinfo, queries,
fragments, wildcard/encoded hosts and whitespace are rejected. Scheme/host and
default ports normalize. See [Runtime setup](../../iris/cockpit/README.md#native-runtime-connection).

Use the returned id in the following routes:

| GET route | Result |
| --- | --- |
| `/api/hale/v1/applications/{id}/capabilities` | Principal mode, implemented reads and explicit command capabilities |
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

Set `HALE_DNA_KNOWLEDGE_URL` to the private state service base URL and configure
`HALE_DNA_KNOWLEDGE_READ_KEY` on both services. The key is at least 32 bytes and
contains no control characters. The public API sends an authenticated native
`POST /graph/read`; it does not invoke a CLI or forward browser authorization
headers. Missing either setting means `reads.knowledge=false` and
`knowledge_unsupported`. Configured failures remain unavailable, never an empty
successful graph. Support is a configuration claim, not a health check.

The API derives the reader from its trusted local configuration or existing OIDC
session before any upstream call. Public queries accept only `id`, `target`,
`limit` (default 25, range 1..100), `cursor` and `snapshot`. Edges, bindings and
dependents require an exact node `id`; nodes allow optional exact lookup. Unknown
or duplicate fields, noncanonical limits and cursor without snapshot fail before
transport. A missing or hidden exact node receives the same 404. A target is a
native locus-path relevance context, not identity or authority.

Knowledge pages use `returned`, `has_more` and `next_cursor`, with no offset or
total. Continue with the same kind, id, target, limit and snapshot. The snapshot
hash binds the Record, projection watermark and graph generation, routed receipt
visibility, reader scope and target. All graph collections for that captured view
share it. A changed view returns 409. The public adapter strictly validates every
nested response, exact Int64 decimal string and the canonical basis hash; it
rejects a wrong Record, reader scope, target or inconsistent page. It rebuilds
public errors with known codes and generic messages, without upstream diagnostic
text or credentials.

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
constructs `ConfiguredKnowledge` from environment settings internally. Response
bodies are capped at 1 MiB and outbound requests at 16 KiB; the private service
also enforces its scan budgets. The public API is source-built; the compiler's
embedded DNA inventory does not currently package this API seed. Packaging a
distributable API belongs to the service deployment work.

Transport deployment prerequisites remain: the current native HTTP client's
`timeout_ms` field is not enforced, so this adapter does not claim a request
deadline. Its `max_retries=0` disables repeat attempts, but the client discards 5xx
response bodies; these become generic `knowledge_unavailable`. An enforced native
transport deadline is required before relying on this path for remote deployment.

## Practice and Review command providers

The standalone executable and `serve` continue to compose `NoCommands`. They
advertise no writes and reject POST with 405. An application can instead call
`serve_with_commands(root, port, web, definitions, commands)` with its native
`CommandProvider`, using the same startup and authentication path. This does not
install a durable command implementation: the application owns it.
Bind the provider to a named locus in the hosting scope and keep it alive for
the server's lifetime before passing it to `serve_with_commands`.

The provider exposes operation/version-aware `supported` and `capability`,
typed `submit(context, CommandRequest)` and shared `lookup(context, request_id)`.
Supported operations occupy the same application/principal/request namespace; reusing
a key with different operation/content must conflict. Trusted context carries
the resolved principal and the application's Record binding.
The HTTP head validates closed versioned command envelopes, byte limits,
expected principal, exact application/subject consistency and browser origin.
The expected principal is a precondition only; it cannot set the authenticated
actor. An identity change returns `command_context_changed` before dispatch.
Unknown fields, caller-supplied authority, duplicate keys and invalid Unicode
are rejected. No CLI command, domain history repair or generic bus publication
occurs in this adapter.

Submission requires JSON Content-Type, `X-Hale-Command: 1`, and an exact Origin
matching the trusted command origin. The loopback composition derives it from
its configured listener; `Api` compositions set `command_origin` explicitly.
Request Host never selects the trusted origin. Missing configuration disables
submission. Duplicate safety headers and ambiguous framing are refused. Recovery
GET accepts exactly one `request_id` and still requires current receipt access.
The command-ID lookup path is not implemented by these profiles.

The provider owns current authorization, canonical request equivalence, durable
principal/application/key scoping, exact-head admission and interrupted proposal
and verdict progression. It must resolve an existing request before applying fresh-subject
preconditions to a retry. Unrelated Record movement is not a browser draft lock;
the provider checks the pinned subject and performs its internal admission CAS.
Receipt lookup joins the exact native request, not the latest practice metadata.

Typed provider results include their captured source basis. The head checks
scope, identity and receipt-state consistency before serialization, and uses
that basis rather than the pre-submit Record head. HTTP 202 means a recorded or
unresolved command state, never Review approval or practice adoption. A successful
proposal proves candidate and Review creation. A successful verdict command proves
that this specific verdict was accepted. Review settlement and activation remain
separate fields in both cases. The [executable wire contract](contract/v1/README.md)
defines the request, response and independent optional capability profiles.

`dna.review.verdict@1` targets an exact pending practice-candidate Review with
separate candidate-digest, expected-principal and pending-state preconditions.
Its literal comment may be empty and is bounded to2048 UTF-8 bytes; supported
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

`tests/commands_api_test.hl` exercises the adapter with scripted providers.
`tests/commands/main.hl` is an explicitly opted-in HTTP conformance fixture,
requiring `HALE_COCKPIT_SCRIPTED_COMMANDS=1`. It appends no native command facts
and provides no restart durability. It must not be used as an application writer
or as proof that the administration loop is complete.

## Handed Tasks and reassignment

`GET /api/hale/v1/applications/{id}/dna/tasks` exposes native human-handoff
history through profile `dna.task-administration.v1`. It is distinct from wf1
workflow execution. Rows retain the outcome, current state and assignee,
obligation, explicit acceptance binding, evidence requirements/reference,
waiting reason and ordered handoff/reassignment history. Acceptance and evidence
references are opaque strings. `assignment_digest` binds the relevant native
lifecycle, including intervening and terminal events; `reassignment_supported`
states factual support, not the caller's authority.

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

The [native command head](practice_review/README.md) can additionally enable
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
under `<root>/.hale/dna/iris/` when the operator wrote none:

```sh
iris/cockpit/start.sh /absolute/path/project --api /absolute/path/practice_review --port 8792
```

`/capabilities` exposes the optional `task_commands` profile
`dna.task.reassign.v1`, explicit `writes.task_reassign`, eligible `recipients`
and separate availability/authorization. Read access or a selected working locus
does not grant this operation. Submit to the shared
`POST /api/hale/v1/applications/{id}/commands` route with the usual Origin,
JSON and `X-Hale-Command: 1` requirements. The closed envelope is:

```json
{
  "request_id": "reassign-1",
  "operation": "dna.task.reassign",
  "operation_version": "1",
  "context": {"application_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "position_id": "org"},
  "target": {"application_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "kind": "dna.task", "id": "task-id"},
  "preconditions": {
    "subject_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "principal": {"mode": "local", "name": "riley"},
    "assignee": "mara"
  },
  "arguments": {"to": "dev"}
}
```

Use the exact returned Task ID, assignment digest and current assignee. The
service checks these and current eligibility in the captured Record, then admits
one `task.reassigned` fact at the exact predecessor. This fact is both the
durable command identity and the assignment effect. The same Task remains open;
its obligation, acceptance, evidence and prior assignment history are preserved.
No Body, Host or Knowledge graph service is needed for this direct operation.
The whole request is bounded to 32768 encoded bytes, request keys to 128 bytes
and person/Task identities to 256 bytes.

The receipt's `task` fields distinguish `applied` from `unknown`, preserving
exact `from`, `to` and native `event_id`. HTTP 202 alone proves no effect. Fresh
Task readback is separate from the recorded receipt and may show later changes.
The application/principal/request key shares the existing Practice/Review/Organization
namespace; different operation or content conflicts. Recover only through
`GET /commands?request_id=...`, with the same application prefix and current
receipt grant. `recover` is independent of `reassign`, so an operator may retain
lookup after new writes are denied. Lookup does not repeat the mutation and
resolves the original receipt even after the Task changes.

This profile does not retire people, transfer responsibility across owners,
modify source ownership, complete Tasks or control workflow execution. OIDC,
signed trust, Ledger-backed administration and source-to-live ownership joins
need their own supported service contract. Existing CLI Task commands are not
made part of this provider by these routes.

## Head: project service

`dna/api/project_service` is the cockpit head (GH #965): the one process the
browser talks to. It serves the shell, owns the operator-machine state — a
project registry, a receipt journal, run and child files under
`${HALE_IRIS_HEAD_STATE:-${XDG_STATE_HOME:-$HOME/.local/state}/hale/iris/head}` —
and reverse-proxies every `/api/hale/v1/applications…` request to the attached
project's API child (`practice_review <root> <api-port>`) on the loopback, so
the browser has one origin and the child's exact-`Origin` guard holds untouched.
Switching a project replaces the child. The head re-implements no verb: each
operation runs `$HALE_BIN dna …` (or `git config` for the two forge keys) as a
detached child under `timeout -k 10 <secs> sh -e`, with its pid, exit and log
as files, so restarting the head interrupts nothing and the next head re-adopts
the children whose command line matches what it recorded.

```sh
hale build dna/api/practice_review
hale build dna/api/project_service
HALE_BIN="$(command -v hale)" \
  ./dna/api/project_service/project_service 8792 iris/cockpit/web dna/api/practice_review/practice_review 8793 [/absolute/path/project]
```

Four routes, described in `contract/v1` beside the Record routes, all under the
head envelope `{"api_version","head":{"profile":"dna.head.v1","principal","active"},"data"}`:

- `GET /api/hale/v1/head` — detached or attached, the active project and its
  API child, the body and observer children, credentials **by name** (what the
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
- `GET /api/hale/v1/head/logs?run=<command_id>|child=api|body|observer[&offset=N]` —
  64 KiB pages of a run's or a child's log.

Operations (all version 1): `dna.project.create` / `init` (600 s runs of
`hale dna new` / `init`, then an attach), `attach` / `detach` / `forget`
(inline; attach validates the worktree, the Record, the `dna/` seed, and
refuses `principal_unsupported` / `trust_unsupported` for OIDC or signed
projects), `dna.project.sync` / `publish` (a commit and push of the genome,
then a sync), `dna.forge.configure` / `sync`, and the sixteen body, secret,
model, connection, handoff and observer operations of the head's contract.
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

`dna.task.create@1` is the cockpit's `hale dna ask`: it records the same
`intent.requested` row the CLI writes, so the host beside the organism relays it
and the organism admits it exactly as it admits a CLI ask. The row's entity is
the intent id (`i` plus the lower-case hex of the current monotonic
milliseconds, as the CLI mints it), its author is the acting principal, and its
body is one flat JSON object whose first three keys are `outcome`, `from` (the
principal) and `to`, followed by the command fields the other operations carry
(`command_format: dna.task-create-command/1`, `command_id`, `command_payload`,
`command_fingerprint`, `command_authority`, `command_authority_basis`,
`command_record_head`). It carries no `via` and no `intent_id`: this head
publishes nothing on the membrane, and the relay splices the id in before the
last brace when it publishes.

Every [native command head](practice_review/README.md) supports it; there is no
policy grant. The authority is the authenticated principal, as with the CLI,
and whether `to` is this organization's to admit is the organism's judgment,
recorded as an `intent.refused` row. `/capabilities` exposes
`task_create_commands` (`dna.task.create.v1`, `max_outcome_bytes` 8192,
`max_identity_bytes` 256, `max_request_bytes` 32768) and `writes.task_create`,
which contributes to `read_only`. Submit to the shared `POST /commands` route
with the usual Origin, JSON and `X-Hale-Command: 1` requirements:

```json
{
  "request_id": "create-1",
  "operation": "dna.task.create",
  "operation_version": "1",
  "context": {"application_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "position_id": "org"},
  "target": {"application_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "kind": "dna.record", "id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
  "preconditions": {
    "record_head": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "principal": {"mode": "local", "name": "riley"}
  },
  "arguments": {"outcome": "Confirm the supplier handover", "to": "org"}
}
```

The target is the Record itself: the Task is minted by the organism after the
ask is admitted, so the request can name only the application. `record_head`
follows the `dna.organization.propose` precedent: the row lands at exactly the
head the request was prepared against, or the request is refused with
`stale_subject` (409) and must be prepared again against the current head.
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
`/dna/tasks`, so `GET /commands?request_id=...` is how the cockpit follows the
ask; nothing else is re-read. After `ledger.adopted` new asks are refused
(`commands_unsupported`) like every other operation here; recorded asks stay
recoverable.

Two asks minted in the same millisecond on one host would share an id and the
organism admits one Task per id; this head steps an id its Record already holds
to the next millisecond, which the CLI does not. A CLI ask beside a cockpit ask
carries no command fields and is invisible to command lookup.

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

The optional cockpit shell and assets are public static content with no Record
data. They can show the sign-in state and independent Runtime connection before
authentication. Record reads still require a valid session in OIDC mode. After
successful sign-in the existing callback redirects to `/`, where the cockpit
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

Set `HALE_IRIS_ORG_DRAFTS=1` on the native API host to offer the optional
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
# Person retirement

The composed local command head accepts an optional `retire` boolean alongside
`reassign` and `recover` in each `dna.task-authority/1` grant. Omission preserves
the existing policy and disables retirement. Keep an explicit `retire: false`
with `recover: true` when revoking writes but retaining request recovery.

`GET /api/hale/v1/applications/{application_id}/dna/people?id=<person>` returns
the exact person state, complete supported responsibility plan, plan digest and
eligible successors. An optional `snapshot` pins its Record head. The read
refuses protected or unsupported affected history instead of reporting a partial
plan. `dna.person.retire@1` uses the shared `/commands` identity namespace, target
`dna.person`, preconditions `{subject_digest, principal}`, and arguments `{to}`.
An empty successor is permitted only for a complete plan with no held Tasks.

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
