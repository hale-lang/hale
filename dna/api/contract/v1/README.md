# Hale API read and optional command adapter contract

This experimental contract describes the DNA reads and optional governance-command
adapter at `/api/hale/v1`. Product scope and acceptance remain tracked in
[#690](https://github.com/hale-lang/hale/issues/690). `openapi.json` lists the
routes; `schema.json` supplies JSON Schema 2020-12 request and response definitions
shared by fixtures and HTTP tests. A default-unavailable provider and scripted
conformance cases do not establish live native durable command completion.

The application id is the native Record genesis identity. Collection item ids
are opaque, including any slashes; use URL encoding for query values. A request
with `id` returns a one-item collection, or 404 when absent. Unknown or repeated
query fields are rejected. The optional command profiles below supply exact
practice replacement and pending practice-candidate verdicts. Broader acting-position authority, remote CLI credentials,
generic runtime observation and recursive execution require their own contracts.

Every successful read response names the local Record identity, head and decimal
revision. This is the inspected local snapshot, not a claim that a remote clone
has synchronized or a Ledger projection is current. Pagination after the first
page requires its snapshot; a changed head returns 409 and the client restarts
the read. No time or global cross-store ordering is inferred from a revision.

`/dna/organization` additionally names its checked source commit, dependency
digest/origin, compiler artifact and static coverage in `data.basis`. Its opaque
snapshot binds those inputs and the Record head. Nodes use exact compiler
instance paths; only an explicit `positions` declaration group marks a role as
`position`. Ownership maps are returned separately without inventing an identity
join or grants. Subscription capacity and supervision retry are decimal strings
or null, preserving values outside JavaScript's exact integer range.

`/dna/definitions` reads an application-injected native `WorkflowCatalog`.
`reads.definitions` is explicit: the standalone head uses `NoWorkflowCatalog`
and advertises false; an application-composed provider advertises support without
promising availability. The response returns all exact definition revisions,
ordered Steps, distinct leaf specifications and child references, and reverse
child dependencies. Leaf-only fields cannot occur on a child member, and a leaf
cannot carry a child reference. IDs remain opaque. Definition revisions and cost
ceilings are canonical signed decimal strings; indices, attempt allowances and
admission limits are canonical unsigned decimal strings. This preserves native
read values without making an admission or execution claim.

Its `data.basis` binds the captured native catalog digest/format, declared
admission limits and loaded-source provenance. `provenance_kind=trusted_host_claims`
is literal: the host supplies the loaded source revision and module, with an
optional dependency digest; these fields are not inferred from current Git HEAD.
The page snapshot binds that basis and the local Record head. A changed catalog,
policy, provenance or Record returns 409. A valid empty catalog succeeds;
unsupported, unavailable, invalid or over-budget catalogs return structured
errors, never partial definitions or a fabricated successful empty result.
Definitions imply no runtime state, authority, Works, Tasks or mutation support.

Practices distinguish proposal, ratification, retirement and decline. A Review
can be settled while its practice is still unratified. `settled` and
`review_settled` preserve native display text. Use `outcome` / `review_outcome`
for a recognized native Review decision, separately from practice activation.
Review reads may additionally include native `is_mutation` (boolean) and
`approvers` (string). The current API emits both; they remain optional in the
schema so legacy read responses stay valid. Their absence disables the restricted
Review decision action rather than assuming a non-mutation or empty quorum.
`author` is the knowledge locus (often `org`); `requester` and `rationale`
attribute the proposal to the person and stated reason. They do not identify
the currently authenticated reader or confer permission to act.

Unavailable text is empty with an explicit availability/status field. The
first slice does not claim complete receipt visibility after Ledger adoption;
it withholds content whose current restrictions cannot be checked from Record,
including after Ledger abandonment, which does not restore those restrictions.
Derived Review questions are subject to the same restriction. It never reads
protected evidence in memory or returns raw review reasoning/diff blobs.
For an ordinary non-knowledge Review, `text_status=not_applicable` indicates
that no practice receipt governs the question declared directly in Record;
`text_available=false` then refers to the absent practice receipt, not the
visibility of that declared question. Known/unknown subject restrictions still
suppress it through the other status values.

Local mode trusts access to the loopback listener. OIDC mode uses the configured
issuer and mapped-member session and fails closed when authentication is absent.
The OpenAPI security alternatives describe these deployment modes; a caller
cannot switch an OIDC service to local mode through a request. This is not a
position-scoped authorization API or a deployment-ready public ingress.

Run the native Hale contract tests from the repository root:

```sh
HALE_API_CONTRACT_ROOT="$PWD/dna/api/contract/v1" \
  hale test dna/api/contract/v1/tests
```

The tests preserve the original nine fixtures plus Organization, Definitions
and Knowledge fixtures. They reject unexplained actor and runtime fields, check
all declared read and command submission/recovery paths, validate request and
response references, and exercise the validator's rejection paths.
Fixture batches validate the complete schema profile once, then strictly parse
and evaluate every example and injected-field mutation. A `Validator` profiles
a schema text the first time it sees it and then only checks documents against
that text; a different text is profiled afresh. The standalone check below
validates one response per run, so it profiles the schema on every run.
Definitions fixtures include leaf/child relationships, multiple exact revisions,
large and signed native values, a valid empty catalog, and structured refusals. To check a captured response:

```sh
hale build dna/api/contract/v1/check
HALE_API_CONTRACT_ROOT="$PWD/dna/api/contract/v1" \
  dna/api/contract/v1/check/check PracticesResponse result.json
```

Running `check/check` without arguments checks the files and fixtures alone.
The live HTTP suite imports `validator` and uses a let-bound instance:

```hale
import "../contract/v1/validator" as wire_contract;

let validator = wire_contract::Validator { root: contract_root };
let result = validator.validate(schema_name, response_json);
```

The result is `ValidationResult { ok, error }`. Missing or malformed schema files
fail the check. No Python packages, other language runtime, network access or
schema downloads are needed.

The native validator implements a **bounded contract profile**, not all of
JSON Schema 2020-12 or OpenAPI. It supports the keywords used here: object,
array, string, integer, boolean and null `type`; `properties`, `required`,
`additionalProperties: false`, `items`; local `#/$defs/Name` references;
string/boolean `const`, unique string `enum`; `minLength` from 0 to 1,000,000;
integer `minimum`/`maximum` paired with `type: integer`; asserted string
`format: int64-decimal` (canonical signed Int64 bounds); the exact canonical unsigned
`^(0|[1-9][0-9]*)$` and signed `^(0|-?[1-9][0-9]*)$` decimal-string patterns; `allOf`, nonempty `anyOf`, and paired `if`/`then`. `$schema`,
`$defs`, `title` and `description` are recognized. Unknown keywords,
unsupported keyword values, unresolved or cyclic references fail before
response validation. Future schema additions therefore need explicit validator
support and tests.

Wire integers use JSON integer tokens (`1`, not `1.0` or `1e0`); comparisons
do not truncate to machine integers. Object keys must be literal, unique and
unescaped, and cannot contain `|`. JSON nesting and schema/evaluation depth
are bounded at 64. String values support UTF-8 and paired Unicode escapes;
`minLength` counts decoded Unicode scalar values. Invalid UTF-8, unpaired
surrogates and escaped U+0000 are rejected; the latter avoids native
NUL-terminated string truncation during constant and enum comparisons.
These narrower wire/profile rules are intentional and do not claim general
JSON Schema conformance. OpenAPI checks cover this release's routes and local
request/response references and the command parameters, not full specification
validation.

## Usage

`Usage` is the object every `Execution`, `ExecutionAttempt` and
`AdministeredTask` carries as `usage` (GH #946): the model calls summed from
their evidence rows. Its four counters (`calls`, `input_tokens`,
`output_tokens`, `cost_micros`) are canonical unsigned decimal strings, never
JSON numbers; `by_position` (at most 64 `UsageByPosition` rows, keyed by the
graph's position id, `position:<name>`) and
`by_backend` (at most 256 `UsageByBackend` rows, keyed `<backend>/<model>`)
repeat the counters under a name. All three objects reject extra fields. The
fixture `usage-summed-by-position-and-backend` is the shape; the three
`invalid-usage-*` fixtures reject an integer counter, an extra breakdown field
and a negative cost.

## The hat and the attempt commands

`ContextResponse` carries a `Hat` (GH #946): one Work's context as structure,
from `GET …/dna/context?id=<work>` (the eighteenth read path). Its `position.id`
is the graph's `position:<name>`; `practices` are `HatPractice` rows (`id`,
`name`, `text`, `kind`, `author`); `history` is `HatFact` rows; `cost_ceiling` and `watermark` are signed
decimal strings; `practices_status` is `resolved`, `no memory` or `unavailable`;
`digest` is sha256 over the body with the digest itself left out. The two
commands are `AttemptClaimCommandRequest` (target `dna.work`; `record_head`
precondition; `capabilities`, `data_classes`, `organizations` as arrays, `ttl`
an integer 1..86400) and `AttemptOutcomeCommandRequest` (target `dna.attempt`;
`holder` and an integer `token` in the preconditions; `disposition` an enum;
`evidence` an array of `AttemptEvidenceCall`; `receipts` an array of
`AttemptReceiptBody`; the hat the leg wore as `hat_digest`, `hat_head`,
`hat_watermark` (integer, -1 for none), `prompt_digest` and `renderer`, optional
in the schema and admitted five together or none), both in `CommandRequest`'s `anyOf`; their receipts
(`AttemptClaimCommandReceipt`, `AttemptOutcomeCommandReceipt`, in
`CommandResponse`'s `anyOf`) carry an `AttemptCommandOutcome` whose `token` is
an unsigned decimal string or `""`. `/capabilities` gains `attempt_commands`
(`AttemptCommandCapabilities`), `writes.attempt_claim`, `writes.attempt_outcome`
and `reads.context`. All of these reject extra fields. Fixtures: the
`attempt-*` and `context-*` cases, valid and refused.

## Knowledge wire additions

`reads.knowledge` is a required boolean. `KnowledgeNodesResponse`,
`KnowledgeEdgesResponse` and `KnowledgeBindingsResponse` cover the four graph
routes (dependents has the binding shape). All objects reject extra fields.
Node `revision` and `ratified_seq` remain signed canonical Int64 strings, including
both extrema; the native validator **asserts** `int64-decimal`. Other validators
must register and assert this format rather than treating it as an annotation.
Projection/routing/visibility counters use nonnegative Int64 strings. Escaped
Unicode values preserve their decoded value; literal, unique keys are required.

Knowledge pagination has `limit`, `returned`, `has_more`, `next_cursor` and
`snapshot`. It has no offset or total. A complete page has a null next cursor;
a continuation has a nonempty cursor and at least one returned row. The public
adapter additionally verifies returned count/limit, exact lookup identity,
Source/basis/reader/target consistency and the canonical snapshot hash. These
cross-field and request-dependent checks are outside the standalone schema.
Cursor requests require the preceding snapshot and unchanged query context.

The basis captures native projection generation/watermark and current routed
visibility, including both Ledger fields after adoption and neither for routing
zero. Coverage reports stored bindings complete and runs, definitions and
practices unavailable. Null `source_provenance` and empty historical metadata
are preserved. Error fixtures distinguish unsupported, unavailable, missing and
changed views without claiming an empty successful result.

## Optional Knowledge relationship command

These profiles implement `dna.knowledge.edge.link@1` and
`dna.knowledge.edge.unlink@1`. Link ensures one directed `(from_id, to_id, rel)`
relationship between exact visible, ratified, unretired Knowledge receipt
identities. Unlink records removal of that exact directed tuple and can clean
relationships involving a retired endpoint when both endpoints remain visible.
Node and binding changes use separate reviewed profiles below. The API admits
these commands in its own process under an explicit authority policy; the
[API README](../../README.md#knowledge-commands) describes that policy, and
the [DNA spec](../../../../spec/dna.md) Record admission and projection.

The application routes are:

- `GET /applications/{application_id}/dna/knowledge/commands/capability[?operation=…]`
- `POST /applications/{application_id}/dna/knowledge/commands`
- `GET /applications/{application_id}/dna/knowledge/commands?request_id=…`

`KnowledgeCommandCapabilityResponse` uses the usual success envelope. Its closed
data object names profile `dna.knowledge.edge.link.v1` or
`dna.knowledge.edge.unlink.v1`, application, authenticated
principal, position `org`, `available`, `authorized`, `mode`, `reason`,
`policy_basis`, recovery `record_lifetime` and decimal byte limits. Submission
requires `available && authorized` with mode `direct` or an implemented `review`
path. The provider owns this choice; a Review-required policy never falls back to
direct admission. The optional `operation` query selects an exact supported operation;
omission defaults to link. Empty, unknown or duplicate fields are rejected.
Permission is operation-specific: omitted `edge_unlink` policy denies removal.
Provider absence preserves read-only behavior. The global capability includes
either authorized available Knowledge operation in `read_only`, while the two existing
Practice/Review write flags and profiles retain their meanings.

`KnowledgeCommandRequest` is the union of closed `KnowledgeLinkCommandRequest`
and `KnowledgeUnlinkCommandRequest` relationship envelopes (plus the node variants
below) containing `request_id`,
`operation`, `operation_version`, `context:{application_id,position_id}`,
`target:{application_id,kind:"dna.knowledge.node",id}`,
`preconditions:{principal:{mode,name},record_head}` and
`arguments:{from_id,to_id,rel,rationale}` for link. Unlink additionally requires
`arguments.edge_id`, equal to the exact graph identity of the directed endpoints
and label. Reversed endpoints or another label do not name the same edge. Link
rejects this removal-only field; its canonical bytes and existing request
fingerprints are unchanged. The target remains a node equal to one endpoint.
The API resolves the actual actor and application; the expected principal is
checked before provider dispatch and never selects authority. POST requires the
configured exact Origin, JSON content type and `X-Hale-Command: 1`, with no query.
Unknown, duplicate and escaped object keys are rejected.

Admission compares the **exact Record head** and appends atomically against it.
Unrelated Record changes therefore invalidate a new request's checked draft;
there is no automatic rebase. Endpoint membership, visibility and authority use
that captured Record. Graph generation and snapshot describe read provenance;
they are not mutation preconditions or a transaction across Record and graph.
The current provider supports complete Record authority with a policy fixed for
its lifetime; new admission after Ledger adoption or abandonment is unavailable.

Request identity is scoped to application, authenticated mode/name and the
Knowledge namespace, separately from the Practice/Review command endpoint.
The fingerprint includes all exact typed fields, including rationale and expected
Record head. Identical retry recovers the existing fact before checking new
eligibility; a changed operation or other validated content under the same key
conflicts. Link and unlink share this namespace. After an
uncertain reply, retain the key and use GET lookup. No POST is automatically retried.

POST returns **202**, and lookup **200**, with `KnowledgeCommandResponse`.
For direct relationship admission its only successful receipt state is `recorded`:
a durable Record effect, not proof of graph projection. Reviewed relationships
use the additional outcome variant below. The receipt names scoped command/request identities,
principal, context, target, fingerprint, `event_id`, decimal-string `sequence`,
`admission_head`, authority and authority basis. Source is the native capture
after the operation; it may be newer during lookup. The adapter checks sequence
against source revision and on POST verifies exact fingerprint, admission head
and visible edge identity. Lookup returns the original stored operation without
an operation query; clients compare it with their saved metadata. These
cross-field checks, digest syntax and UTF-8 byte
bounds remain native adapter checks outside the limited schema profile.

Current recovery authority is checked independently. If endpoint details are
unavailable, `details_visible=false` requires empty `target.id` and `edge_id`.
The receipt never echoes rationale, endpoint labels or content. A fresh graph
read is required to establish observation. A removal receipt does not prove the
tuple previously existed or is absent from the current graph. Observing absence
requires complete applicable edge-page coverage at one fresh snapshot; hidden,
failed or truncated reads cannot establish removal. Projection failure cannot undo the
recorded effect. Direct admission uncertainty is a 503 error; reviewed proposals may
report `outcome_unknown` when their durable request is known but later evidence is
ambiguous.

Bounds are UTF-8 bytes: request ID 128; application/principal/Record head 256;
relationship label 256; rationale 2048; encoded HTTP body 32768. Identifiers and
labels exclude control characters; rationale preserves non-NUL controls and
Unicode. Endpoints are `sha256:` plus 64 lowercase hex digits. Stale subject,
request conflict or principal change returns 409; policy denial 403; absent
receipt 404; unsupported, unavailable, busy, invalid history or uncertain outcome 503.

The API admits commands in its own process, under the authority policy
`HALE_DNA_KNOWLEDGE_COMMAND_POLICY` names, read once at startup; there is no
Knowledge service, URL or command key. Private capability, submit and lookup
keep their encoded requests with trusted context and typed results, handled
in-process. Private capability accepts an optional operation alongside context;
link retains the original context-only body and unlink explicitly names its
operation. Private fallback errors use a 200 envelope so that typed codes such as
Review-required and uncertain outcome survive; the public API maps the typed
error to its actual 503 status, distinct from an authenticated actor's policy
denial 403.

## Optional practice replacement command profile

`Capabilities.commands` is optional. With no other command provider, its absence
preserves the original read-only capability response with both write flags false. A present
`PracticeCommandCapabilities` object has the exact profile
`dna.practice.propose.v1`, separate `available`/`authorized` booleans, position
`org`, recovery `record_lifetime`, fixed decimal byte limits `8192`/`2048` and a
reason. The API advertises `writes.practice_propose=true` only when the provider
is available, submission is authorized and a permitted command origin is
configured. Without the optional Review profile, `review_verdict` remains false.
Globally, `read_only=false` means at least one configured operation is currently
available and authorized, including the separately discovered Knowledge command
profile. Either named Practice/Review write flag implies `read_only=false`;
both flags being false does not imply `read_only=true`. Available
and authorized may coexist with writes disabled when origin configuration is
missing. Receipt lookup permission is checked independently of write permission.

`POST /applications/{application_id}/commands` accepts a closed
`PracticeCommandRequest`: caller-stable `request_id`, operation
`dna.practice.propose`, string version `1`, application/`org` context, exact
`dna.practice` target, `preconditions.subject_digest`, the required closed
`preconditions.principal` object with `mode` (`local` or `oidc`) and nonempty
`name`, and nonempty replacement `text`/`rationale` arguments. It has no query
parameters. This principal is an untrusted expected identity. The adapter compares
both fields with its authenticated context before any provider capability or
submit call; a mismatch returns 409 `command_context_changed` with no provider
dispatch. The precondition never selects the actor or grants authority. Caller
author, role and principal fields outside this precondition remain forbidden.
The provider obtains the practice name and supersedes metadata from the pinned
predecessor. Native admission, authority, request equivalence and exact-head
compare-and-append remain provider duties. Unrelated Record advancement is not
a public draft precondition.

The adapter checks both application fields against the authenticated route's
Record and target ID against subject digest. It enforces 1–128 UTF-8 bytes for
request IDs, 1–256 for application/resource/digest IDs and principal names, no
control characters in those fields, at most 8192 bytes of text, 2048 of rationale
and 32768 for the whole body. Resource IDs remain opaque at this adapter; native
digest syntax and predecessor eligibility belong to the provider.
Decoded text/rationale are preserved without trimming or truncation. These byte
bounds and cross-field equality checks are imperative adapter checks, outside
this schema profile. Malformed Unicode and unknown, duplicate or escaped object
keys are rejected by the native wire validator.

Submission requires JSON Content-Type, `X-Hale-Command: 1` and exactly the
explicitly configured permitted `Origin`; the API never derives this permission
from request Host. Header handling follows HTTP rules. Empty, null or multiple
origins fail; no CORS is supplied. Local/OIDC identity resolution precedes
dispatch. Missing origin configuration disables submission even with a provider.
The default absent provider may retain the historical POST 405 refusal; a
supported provider with unavailable submission returns a structured 503.

For this operation, `CommandResponse` wraps a closed `PracticeCommandReceipt` with `api_version`
and the provider's captured `source` basis. The adapter validates that basis and
all identities against the trusted context/request; it must not substitute a
pre-submit head or manufacture a receipt. Command states are `recorded`,
`admitted`, `running`, `succeeded`, `refused`, `failed` and `outcome_unknown`.
POST returns 202 for the first three and `outcome_unknown`, and 200 for terminal
receipt states. Errors use the existing `ErrorResponse` wrapper and the status
codes declared in OpenAPI.

Proposal, Review and activation remain separate fields. A created proposal has
an exact candidate digest and Review ID; command success requires creation and
command refusal requires proposal refusal. Pending/settled Review requires a
created proposal and its exact candidate as subject. The schema checks required
states and nonempty values; the adapter checks candidate/subject equality.
Unavailable Review has empty outcome and subject; settled Review has a recognized
outcome. Adopted activation requires settled approval. Approval may coexist with
pending, refused or unknown activation and never establishes adoption by itself.

`GET` on the same command path requires exactly one nonempty `request_id` query
value and returns 200 for any valid receipt state. It is an authenticated read
that cannot append or execute. Recovery uses the same caller key after ambiguous
transport, authentication loss or reload. Unavailable/unknown lookup and 404 do
not establish that an in-flight submission cannot later be accepted; retain the
key and allow lookup without automatic resubmission. An identity-change refusal
clears in-memory drafts/results while retaining the scoped recovery identity;
it never triggers submission under the newly authenticated principal. Contract
fixtures cover these wire states, strict expected-identity envelopes and
approval/adoption separation. They do
not prove native durability, authorization or restart recovery; those require
the upstream provider and a separate live integration gate.

## Reviewed Knowledge relationships

The edge profiles also support policy-selected Review admission without changing
request arguments, canonical bytes, fingerprints, request identity or limits.
Direct receipts retain their exact existing fields. A reviewed edge receipt adds
one required, closed `relationship` outcome object; no `reviewed` boolean appears
on either wire. The private receipt has 19 fields on this branch and 18 for direct
receipts. Recovery uses the original fact path, even if current policy mode changes.
The presence of `relationship`, rather than the operation or current capability,
is the receipt discriminator. Node/binding outcomes cannot accompany it.

The eight fields are `proposal_state`, `candidate_digest`, `review_id`,
`review_state`, `review_outcome`, `effect_state`, `effect_reason`, and `reason`.
Proposal pending/created/refused/unknown maps to outer
recorded/succeeded/refused/outcome_unknown. Created supplies exact candidate and
Review identities; it means proposal creation, not an applied relationship.
Visible receipts retain the exact top-level `edge_id` at every stage, including
pending and refused. Hidden receipts retain the relationship object and durable
outer state but clear target/edge/candidate/Review identifiers and reasons, with
unknown proposal/effect and unavailable Review.

Effects are unknown/pending/linked/unlinked/declined/refused. Linked and unlinked
must match the submitted operation and an exact approved Review. Refused means
an approved native effect was refused. Declined requires an explicit native
reject/revise consequence. Pending can follow any settled verdict while its
consequence remains unestablished. Bounded service explanations are permitted for
declined and refused; other effect stages have an empty explanation. Approval,
Record effect, and fresh graph observation remain separate evidence.

The existing `/dna/reviews/candidate?id=<Review>&snapshot=<Record-head>` returns
`kind:"knowledge_edge_change"` and the exact canonical JSON string described by
`KnowledgeEdgeCandidateDocument`. Its 14 closed fields pin application, scoped
request, command fingerprint, captured grant, operation, edge ID, ordered
endpoints, literal relation, proposer, rationale and exact tuple basis. The native
reader verifies the original admitted fact, application genesis and Record row
author, document hash, Body progression, Review association and both endpoint
receipts. Protected candidate/either endpoint yields the same 404 as missing.
Historical settled candidates and retired-but-readable endpoints remain readable.

Relationship Reviews expose `knowledge_edge_digest` and actual proposer `author`,
with empty node/binding discriminators and no invented target/class. The existing
exact verdict profile enforces pending subject identity, independent reviewer and
board authority. Its node activation stays unknown; the original Knowledge
receipt reports the relationship effect. A captured per-tuple `edge_basis`
prevents later direct or reviewed changes, including ABA, from being overwritten
by delayed approval. It does not introduce a graph-generation precondition or
invalidate a candidate merely because unrelated Record rows were appended.

## Optional Knowledge node commands

The same Knowledge submit/lookup namespace additionally supports
`dna.knowledge.node.propose@1`, `dna.knowledge.node.revise@1` and
`dna.knowledge.node.retire@1`. Select the corresponding capability operation;
the profile is the operation followed by `.v1`. Node capabilities require
`available && authorized && mode === "review"`: this is the implemented native
proposal/Review path, independently authorized from relationship policies.
They advertise `max_text_bytes="8192"`, `max_rationale_bytes="2048"` and
`max_request_bytes="98304"`, replacing the edge-only relationship limit.
Any available authorized node operation makes global `read_only=false`.

All variants keep the existing application, position `org`, expected principal,
request identity and exact Record-head precondition. Propose arguments are
`{kind,name,text,author,target,rationale}` and the envelope target is
`{application_id,kind:"dna.knowledge.collection",id:arguments.target}`.
Revise adds `arguments.supersedes` and targets that exact node digest. Retire
accepts only `{id,rationale}` and targets that exact node; the command admission
derives its retirement proposal from the canonical predecessor. Propose/revise
kinds are `idea`, `concept`, `practice` and `task_concept`; callers cannot supply
kind `retirement`. New proposal/revision names are nonempty, at most 256 UTF-8
bytes. Existing unnamed candidates remain readable and reviewable, and may be
retired. Policy explicitly grants author/target
pairs, and revision requires both predecessor and requested pairs to be granted.
Position `org` in the API context does not rewrite the requested binding target.

Node requests use the same per-application/principal Knowledge request namespace
as relationships. Changed operation or validated content conflicts under an
existing key. Public and private node requests have a 98304-byte encoded budget;
edge requests retain 32768. Decoded text remains at most 8192 UTF-8 bytes and
rationale 2048. Canonical payload identity preserves Unicode, CRLF and non-NUL
controls. A recorded admission is not a created candidate or an adopted node.

Node receipts preserve common identities and Record evidence, have empty
`edge_id`, and add the closed `node` outcome object. `proposal_state=pending`
corresponds to outer `state=recorded`; `created` to `succeeded`; `refused` to
`refused`; `unknown` to `outcome_unknown`. Created supplies exact
`candidate_digest` and `review_id`. `review_state` is unavailable/pending/settled,
with approve/reject/revise only when settled. `activation_state` is
unknown/pending/adopted/refused; approval may succeed while adoption is refused.
Only exact candidate ratification plus any required predecessor retirement proves
adoption. `reason` and `activation_reason` are service-controlled summaries.
Hidden details clear target/candidate/Review identities and expose unknown proposal,
unavailable Review and unknown activation with empty reasons. GET recovery checks
current authority; graph projection is established separately by fresh reads.

Exact `GET /dna/practices?id=<digest>` also reads recognized unnamed Knowledge
and retirement candidates using the existing row shape and canonical receipt
validation. It does not add those candidates to the named Practice list. Generic
Review pages use this exact source-bound document, not graph projection text.

## Optional reviewed binding commands

`dna.knowledge.binding.bind@1` and `dna.knowledge.binding.unbind@1` use the same
Knowledge submit/lookup namespace, authenticated principal, position `org`, and
exact Record-head admission precondition. Both target the exact underlying item
with `{application_id,kind:"dna.knowledge.node",id:idea_id}`. Bind arguments are
`{idea_id,author,target,rationale}`; unbind additionally requires the exact existing
`binding_id`. The native tuple identity remains the digest of `[idea_id,target,author]`.
Callers cannot supply derived class or applicability. `author` and `target` are
requested loci, never principal or authority overrides. Lateral pairs are refused.

Each capability uses its operation plus `.v1`, `mode="review"`,
`max_locus_bytes="256"`, `max_rationale_bytes="2048"`, and
`max_request_bytes="32768"`. Both available and authorized must hold. Either
profile makes global `read_only=false`. Missing policy grants deny; the owning
policy grants each operation and exact author/target pairs independently. Bind
requires a visible active ratified item; unbind permits a visible retired item so
historical applicability can be removed. All Knowledge operations share request
identity: reuse with different operation or validated content conflicts.

Binding receipts retain common identities and Record evidence, empty `edge_id`,
and a closed `binding` object containing `binding_id`, `proposal_state`,
`candidate_digest`, `review_id`, `review_state`, `review_outcome`, `effect_state`,
`effect_reason`, and `reason`. Proposal stages match node receipts. A visible
receipt always retains its exact binding identity, even before candidate creation.
Effects are `unknown`, `pending`, `bound`, `unbound`, `declined`, or `refused`.
`bound`/`unbound` require the corresponding native operation and exact approved
Review plus native effect; `refused` distinguishes an approved but refused effect.
`declined` requires an explicit rejection/revision terminal fact. Review approval
alone proves neither a binding effect nor graph observation. Hidden receipts clear
all target/binding/candidate/Review identifiers and reasons and expose unknown
proposal/effect and unavailable Review. Reasons are service-controlled summaries.

A binding Review has `knowledge_binding_digest`, empty `knowledge_digest`, and
an exact canonical JSON document, not a Knowledge node. Read it with
`GET /dna/reviews/candidate?id=<review_id>&snapshot=<Record-head>`. Both query
fields are required; pagination and extra fields are rejected. The ordinary
success envelope contains `data:{kind:"knowledge_binding_change",review_id,
candidate_digest,document}`, where `document` is the exact canonical JSON string.
`KnowledgeBindingCandidateDocument` describes its 15 closed fields. The reader
verifies the original admitted command, captured grant, Record application and
row author, tuple/basis, exact document digest, and native Review association.
The underlying item's current receipt visibility gates this read. Protected and
missing candidates return identical 404 responses; a stale source returns 409.
Settled readable Reviews retain this historical canonical read.

The Review's `author` is the actual proposer (`document.by`), while `target` and
`binding_class` match the canonical document. Hidden binding Reviews mask those
scope-derived values and prose. The existing verdict profile accepts eligible
pending binding Reviews using the same exact-subject, board authority and
independence checks. Its activation remains unknown: binding effects belong to
the original Knowledge command receipt. At effect time, the native per-tuple
`binding_basis` guard refuses intervening/ABA tuple changes. Unrelated Record
movement does not invalidate a pending Review. These are Record-owned decisions;
there is no atomic Record/graph promise. Fresh graph reads establish observation.

## Optional exact-candidate Review verdict profile

`Capabilities.review_commands` is an optional sibling of the unchanged practice
`commands` object. Its closed `ReviewCommandCapabilities` shape contains profile
`dna.review.verdict.v1`, independent `available` and `authorized` flags, position
`org`, recovery `record_lifetime`, `max_comment_bytes="2048"` and a reason.
`writes.review_verdict` requires that matching profile, availability,
authorization and the configured trusted origin. Practice and Review writes are
independent; either enabled write makes `read_only=false`. The separate Knowledge
command capability may also make the application writable. Neither profile
alone enables the other operation. Existing practice-only payloads remain valid;
clients must assess each supported profile independently.

The same POST path accepts `CommandRequest`, an operation-discriminated union of
the unchanged `PracticeCommandRequest` and `ReviewCommandRequest`. The latter
uses operation `dna.review.verdict`, version `1`, application/`org` context and
target kind `dna.review`. Its opaque target ID identifies the Review, separately
from `preconditions.subject_digest`, which identifies the exact candidate.
Preconditions also require the existing expected-principal object and literal
`review_state="pending"`. Closed arguments contain `verdict` (`approve`, `reject`
or `revise`) and `comment`. The comment is required, may be empty, and preserves
literal decoded text up to 2048 UTF-8 bytes. The existing identity/body bounds,
strict keys and Unicode, header/origin checks and principal comparison apply.
Caller-supplied reviewer/authority and abstention are not supported.

Eligibility is imperative provider/browser behavior. This profile covers readable,
pending Knowledge candidates of kind idea/concept/practice/task_concept/retirement,
and the separately typed binding and relationship candidates described above. For node candidates,
The browser reads the canonical candidate at the Review's exact Source/snapshot and requires
`review.subject_digest == review.knowledge_digest == candidate.id == candidate.digest`,
`candidate.review_id == review.id`, pending readable Review/candidate state,
matching candidate and Review author/target, explicit `review.is_mutation == false`, empty
`review.approvers` and `review.required_authority == "board"`. The latter is a
restriction of this first practice-review profile, never proof of the current
principal's permission. Missing legacy metadata disables the action while reads
remain available. Ordinary, unlinked, mutation, owner-approval,
settled and protected Reviews cannot enable submission. The provider must recheck
pending state, exact subject, eligibility, readability, authority and independence
at the authoritative delayed decision, preventing settled-Review reapplication.
`required_authority` is information rather than a grant. No global Record-head
precondition substitutes for these checks.

`ReviewCommandReceipt` retains the common command identity, principal, context,
target, subject, fingerprint, state and provider-captured Source. It replaces
`proposal` with closed `verdict:{value,state}`, where value is the submitted
choice and state is `pending`, `accepted`, `refused` or `unknown`. Command success
requires this verdict's acceptance; command refusal requires its refusal. The
adapter checks the returned choice against the POST argument. GET validates the
returned operation variant without reconstructing a request body.

Observed `review` and `activation` retain their existing state sets. An accepted
verdict may leave the Review pending. A refused verdict may coexist with another
command's settled approval or adoption. Pending/settled Review subject must equal
the exact candidate; unavailable Review has empty outcome and subject. Adoption
requires exact-subject settled approval. Reject/revise does not fabricate an
activation refusal. Schema checks shape and state implications; the adapter
checks identity and subject equality. An observed Review settlement never proves
this command's verdict result.

One operation-aware provider owns the shared application/principal/request-ID
namespace and unchanged `GET /commands?request_id=…` lookup. Reusing a key for a
different operation conflicts. Review ID cannot replace a durable command
identity or correlate one of several verdict commands. The default executable
uses `NoCommands`; scripted conformance does not prove native attribution,
durability or recovery.

The browser retains one existing recovery storage key and Web Lock per
application/principal across both operations. Legacy version-1 metadata remains
practice metadata. Closed version-2 metadata adds operation, operation version,
position and target kind to the existing application/principal/request/target/
subject fields; it stores no text, decision note, verdict draft or receipt.
Unknown versions stay retained and block submission. Recovery remains available
during domain-read outages and write revocation, never resubmits POST or clears
an unresolved key after 404. Explicit completed-request dismissal precedes the
next operation.

The optional `reads.workflows` capability identifies the trusted-local recorded
execution reader. `WorkflowsResponse` keeps list summaries separate from exact
recipe nodes, attempts and transition history. `maxItems` is supported by the
native schema validator for bounded arrays. Record snapshots pin pagination;
this contract currently declares `memory: record`, `routing: "0"` and no runtime
association. It does not advertise live execution, Ledger read completeness or
hosted visibility. A `task.born` fact without a wf1 admission/refusal is explicitly
unclassified, because a birth alone cannot identify its execution engine.

`OwnershipDraftRequest`/`OwnershipDraftResponse` describe optional preparation of
the fixed `dna/org/owners` file. `ownership_drafts` advertises the bounded profile;
GET returns `parsed_source`, POST returns `valid_draft`, and publication is always
unavailable. The native adapter verifies exact bytes, digest, principal and source
basis in addition to schema shape. Impact comes from the native ownership model,
not a browser-generated policy or a live-work inventory. Hosting/mode changes,
affected-owner review membership and before/after inherited owners remain explicit.

### Immutable Organization source candidates

The exact `/dna/reviews/candidate?id=…&snapshot=…` read also accepts retained native Organization source Reviews. Its `organization_source_change` variant contains `review_id`, `candidate_commit` and a literal JSON `document` described by `OrganizationSourceCandidateDocument`. The candidate identity is a Git commit; the fixed `dna/org/main.hl` module has its separate SHA256 digest. The reader verifies the original admitted request, native preparation/verification/Review facts, one exact base parent, retained candidate reference, unchanged other tracked paths, and exact bounded module bytes. It remains usable after the candidate worktree disappears or the destination checkout advances.

Source evidence requires the explicitly injected Organization authority's **recovery grant**: `authorize(context, "dna.organization.propose", true, journal)`. This is the source evidence read/recovery permission, defaults denied, and permits an independent reviewer with `recover=true` and `organization_propose=false`. It does not borrow Practice/Knowledge permissions or enable public Organization publishing. `serve_with_source_evidence` is the explicit composition seam; existing `serve`/`serve_with_commands` callers retain default denial.

The returned module before/after strings preserve exact UTF-8 and line endings (each at most 16 KiB natively). Verification includes a deliberately selected `semantic_diff` projection with classification, summary counts and declaration identity/change. It omits compiler locations and raw contract/effect/law payloads and diagnostics. `receipt_digest` identifies the original retained native JSON receipt; `digest` identifies the **different returned projection** document. Both retained native diff receipts are bounded to 128 KiB and verified by digest before projection. Unsupported/incomplete semantic evidence is unavailable rather than invented.

Existing receipt restrictions apply to both source modules, base/candidate identities and diff receipts. Missing and withheld exact candidates share 404. Ordinary Review reads additionally emit `organization_source:true` plus source request/digest links when readable; denied or withheld source rows clear those links, author/owners, question and settlement prose. The final Record/candidate/receipt reference fence rejects movement with retryable `snapshot_changed`. The read reports no source application, restart or activation outcome.

### Optional Organization commands

The existing `/commands` namespace now has a typed `dna.organization.propose@1` request and receipt variant. The request selects only `dna/org/main.hl`, with `arguments:{source_text,rationale}` and `preconditions:{principal,base}`; `base` is the exact five-field captured source/module/dependency/Record basis. Native limits are 16 KiB decoded UTF-8 source, 2048 rationale bytes and an independent 32 KiB encoded request. Principal is an expectation checked against authentication, never an authority source. The existing request identity namespace, exact-key lookup and Practice/Review request bytes are unchanged.

An explicitly configured provider may expose `organization_commands` (`dna.organization.propose.v1`) and `organization_review_commands` (`dna.organization.review.verdict.v1`). Both separate availability from authorization and include the exact `command_origin`. Optional `writes.organization_propose` and `writes.organization_review_verdict` contribute to `read_only`. An ordinary `review_commands` grant does not authorize source Review: the explicit source profile is required. Source Review still submits the existing `dna.review.verdict` request and receives the existing verdict receipt shape, preserving native mutation/owner-quorum semantics.

The Organization receipt has an `organization` object containing `proposal_state`, `source_head`, `source_digest`, `mutation_id`, `candidate_commit`, `application_state`, `application_reason_code`, and `restart_handoff_state`; it has no Practice `proposal` or decision `verdict` branch. `proposal_state:created` / outer `state:succeeded` means the prepared native candidate and Review exist. Later approved-but-refused application, rejected/revised change, unknown application, and requested restart handoff remain separate facts. Native verification failure is `proposal_state:failed` with outer `state:refused`. This profile requires `activation:{state:"unknown",reason:""}`; applying source or requesting a restart does not prove a running candidate.

The composed head optionally reads `HALE_DNA_ORGANIZATION_POLICY` as a separately validated, application-bound `dna.organization-authority/1` policy and uses that same named authority for native commands and immutable source evidence. Both source write enablement flags remain **off** in the head; merely setting the policy enables permitted recovery/evidence reads and advertises unavailable source write profiles. Malformed configured policy refuses startup. The provider's lookup continues to enforce the original stored command's current recovery permission when new writes are disabled. Public source publication awaits the owning deployment's impact and delivery integration.

### Authorized Organization source status

`GET /dna/organization/source-status?id=<exact Review/mutation ID>` lets an independent reviewer follow the same source change without the proposer's private request key. Optional `snapshot` pins the current Record; omission captures fresh status. The separately configured Organization recovery grant is also the source evidence read grant, so `recover=true` can permit this read with `organization_propose=false`. Default composition denies source reads. The exact candidate and retained source/semantic receipts retain their existing visibility restrictions.

`OrganizationSourceStatusResponse` separates source identity, Review/quorum, application, restart handoff, native launch, historical observation, exit and rollback/base launch. It returns selected typed facts and sanitized reason codes, never source request inputs, local paths or lease data. A historical healthy host observation precedes its separate native observation-delivery acknowledgment and does not prove retention or current running. Conflicting stage evidence is unknown; missing evidence is unavailable. Source and command receipt formats remain unchanged.

`current_running` is unavailable by default. An explicitly injected `OrganizationRuntimeProvider` must prove the exact current attempt, launch, candidate, executable/topology and opaque process key; the reader checks those identities and the final Record/candidate fence. The opaque key allows navigation to the existing Runtime inspector. No PID is exposed, and a matching binary alone is insufficient. Public source writes remain disabled; this read does not resolve the outstanding live-obligation identity and admission-fence requirements for full Organization publication.

Organization source status uses retained exact source objects independently of the candidate renderer’s 128 KiB diff receipt limit: no diff content is returned by status. Protected supporting Review or application Event digests hide the whole status resource.

### Current Organization responsibility evidence

`GET /dna/organization/source-impact?id=<exact Review/mutation ID>&snapshot=<Record head>` reads current recorded responsibilities for that retained source candidate. Only `id` and optional `snapshot` are accepted. It requires the separate source evidence/recovery grant and currently supports the local Record profile. Source comparison text need not be renderable; exact retained source objects still establish the candidate association. Hidden candidate support makes the resource unavailable.

`OrganizationSourceImpactResponse` carries the exact source identities, captured Record basis, an application-wide state and eleven separate decimal-string counts. `obligations_observed` and `none_observed` apply only to the bounded supported history. `unavailable` carries `counts:null`; incomplete coverage, unsupported operational kinds and protected operational references cannot expose partial counts. These categories overlap and must not be summed. In addition to schema validation, consumers enforce open-workflow Tasks ≤ workflow Tasks, open-legacy Tasks ≤ legacy Tasks, and handed Tasks ≤ open-legacy Tasks.

The native reader reuses wf1 projection and selected recorded legacy Task, intent, schedule and effect semantics. Limits include 2048 Record rows, 512 operational facts, 256 bound workflow nodes and 512 legacy identities. Ordinary Host `body.claimed` records and exactly correlated healthy `pressure.remeasured` summaries are recognized as historical diagnostics; neither closes work nor establishes current ownership. Forced claims and unproven diagnostic summaries remain unsupported. Any prior Ledger adoption, other unsupported history such as model calls, or uninspectable Knowledge bindings prevents a complete public assessment. The response provides no position attribution, per-person task list or deployment authority.

This is a current read, not an assessment retained at the earlier Review decision. `position_binding` and `admission_fence` remain explicitly unavailable. A final source application still requires the owning service to establish complete impact under an operational admission barrier; this endpoint neither acquires that barrier nor enables publishing.

### Exact recorded Task assignee

Only `GET /dna/tasks` accepts optional `assignee` alongside the common exact-id/pagination/snapshot query. The decoded identity is nonempty, at most 256 UTF-8 bytes and contains no C0/DEL; duplicate, empty or malformed fields return 400. Filtering precedes counts and pagination, uses the latest recorded assignee and retains terminal-after-handoff history. `id` and `assignee` must both match or return the same 404 as an absent/protected Task. `TaskAdministration.assignee` is present only for a filtered response and echoes the requested exact identity; consumers also verify every returned row has that assignee. Unfiltered wire shape is unchanged. Assignment is not owner membership, source position or command authority.
### Raising work: `dna.task.create@1`

`TaskCreateCommandRequest` raises work the way `hale dna ask` does. Its
`arguments` are `outcome` (natively at most 8192 UTF-8 bytes, control bytes
preserved) and `to` (a position, 1..256 bytes); its `preconditions` are the
expected `principal` and the `record_head` the request was prepared against.
The target is the Record itself (`kind: dna.record`, `id` = application id):
the Task is minted by the organism, not by this request. No intent id, asker,
`via` or Task id is caller-supplied. The shared request identity namespace,
exact-key lookup and the other operations' bytes are unchanged.

`task_create_commands` advertises `dna.task.create.v1` with `writes.task_create`,
which contributes to `read_only`. There is no policy grant: any authenticated
principal may ask, as with the CLI, and whether the position is this
organization's to admit is the organism's judgment.

`TaskCreateCommandReceipt` succeeds once the `intent.requested` row is in the
Record; a stale `record_head` is refused with `stale_subject`. `subject_digest`
is that Record head. `task_create` names the minted `intent_id` and the row's
`event_id`, and re-derives the organism's answer on every lookup:
`intent_state` is `requested` until it answers, then `offered`, `refused`, or
`born` with the `task_id`. An uncertain append is `outcome_unknown` with
`intent_state: unknown` and no intent. A born-but-unhanded Task is not yet a
handed-Task read; the receipt is how the face follows it.

# Person retirement contract

The optional `person_commands` capability advertises `dna.person.retire.v1` and
the independent `writes.person_retire` grant. `reads.people` enables exact person
inspection at `/dna/people?id=<person>`. The read includes no inferred membership
or position authority. Its complete bounded responsibility plan is tied to one
Record head; a changed snapshot returns 409 and an unsupported/protected plan is
unavailable without partial counts.

`PersonCommandRequest` and `PersonCommandReceipt` extend the existing shared
command unions. The request carries a plan digest and successor; the receipt
identifies the exact retirement event and decimal transferred count, or an
unknown outcome with no event/count. Native admission checks the full plan and
publishes ordinary reassignment facts and retirement together. Empty successors
require zero held Tasks. The local profile supports at most 32 transfers and
requires updated Body/CLI writers; it does not claim a cross-store transaction.

## Head

The four `/api/hale/v1/head…` paths belong to the head (GH #965), the
trusted-local project service in front of a per-project Record API. They use
the `HeadEnvelope` shape — `api_version`, `head:{profile:"dna.head.v1",
principal, active}` and `data` — not a Record `source`, because a head read has
no Record identity of its own; a head envelope carrying `source` is rejected.
`HeadResponse` is the head's state (detached or attached, the active project
and its API child, the body child, credentials **by name only**,
the running receipt and every operation's availability); `HeadProjectsResponse`
the registry with per-project detail fields on the exact `?id=` read;
`HeadLogResponse` one bounded page of a run's or a child's log.

`HeadCommandRequest` is one closed envelope for all twenty-three operations:
`operation` is the fixed enumeration, `operation_version` the string `"1"`,
`context.head` the constant `local`, `target` is `dna.head`/`local` for the
four head-scoped operations and `dna.project`/`<application_id>` otherwise,
and `arguments` is the closed superset of every operation's arguments — each
operation admits only its own keys natively, and no operation admits `value`:
a secret names a `SecretSource` (`file` or `env`, by name). `HeadCommandReceipt`
folds the receipt journal: `recorded`, `admitted`, `refused`, `running`,
`succeeded`, `failed` or `outcome_unknown`, with `run` (`run`, `body` or
`inline`; `null` before execution and on a refusal) and an
operation-specific `outcome` that carries `record:{head_before,head_after,rows}`
on row-writing operations. Submission answers 200 on a terminal receipt and 202
otherwise; recovery is `GET …?request_id=` under the head's own principal.

`openapi.json` pins 26 paths: the head command path mirrors the Record command
path (required `Origin` and `X-Hale-Command` headers, a required
`HeadCommandRequest` body, 202 declared on submission only, recovery without a
body), the projects read declares one optional `id`, and the logs read
`run`, `child` and `offset`. Fixtures cover a detached and an attached state,
the registry list and detail, create, attach, sync, publish, secret-by-source
and confirmed-probe requests, running, succeeded, refused and unknown-outcome
receipts and a log page, plus the negatives: a secret request carrying a
value, an unknown state, a head envelope with `source`, an injected `actor`
and a numeric `operation_version`. Scripted conformance proves the wire shape,
not the native head's durable admission or recovery.
