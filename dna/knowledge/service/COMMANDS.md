# Native Knowledge commands

The Knowledge service owns `dna.knowledge.edge.link@1` and
`dna.knowledge.edge.unlink@1`. An application may explicitly grant either
operation independently. Link ensures the directed relationship
`(from_id, to_id, rel)` between two visible, ratified, unretired Knowledge nodes.
Unlink ensures that exact tuple is absent. It also requires its exact graph
`edge_id`; reversing the endpoints or changing the label cannot remove a different
relationship. Visible retired nodes remain eligible for historical link removal.
An already absent tuple is a safe no-op in the projection; its admitted directive
still has a durable request receipt.
Self-links and different labels remain distinct relationships. This operation
does not change a node, its locus bindings, or its governance classification.

In direct mode, the command and its attribution are one `knowledge.edge.linked` or
`knowledge.edge.unlinked` Record fact.
The graph is a projection of that fact. The existing projection checkpoint
protocol withholds incomplete projections and replays an interrupted suffix.
Rebuilding an empty graph from the Record restores the ordered final state,
including link → unlink → relink. An existing link or absent unlink does not
change graph rows; advancing projection provenance still changes the generation. A command receipt reports
`recorded`. Iris separately reads the graph to report whether it has observed
that effect. Removal observation requires a complete same-snapshot traversal of
the selected node’s relationships and a fresh receipt lookup proving endpoint
visibility at the same Record head. A partial page, changed basis, restricted
endpoint or unavailable service cannot prove absence. Graph unavailability does
not turn a committed effect into failure. A later link may restore the same tuple
without changing the earlier removal receipt.

## Deployment

Build the matching native API and Knowledge service sources. Keep the existing
Knowledge URL, read key, DSN and Record configuration. To enable commands, set:

- `HALE_DNA_KNOWLEDGE_COMMAND_KEY` in the API and Knowledge service. It must be
  32–4096 bytes and differ from the read key. Neither key reaches the browser.
- `HALE_DNA_KNOWLEDGE_COMMAND_POLICY` in the Knowledge service, naming its explicit
  authority document. The policy is validated and captured at startup.

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

Grants match the exact authenticated mode/name pair. `edge_link` and optional
`edge_unlink` are `direct`, `review`, or `deny`. Omitting `edge_unlink` denies
removal, so existing link-only policies remain unchanged. A `review` policy
admits a native relationship proposal through the path below. The provider never
substitutes a direct effect. Recovery permission is separate from write
permission. Missing policy denies commands. A retired person cannot submit or
recover commands. The service honors the Record's configured signing mode.

This version requires a complete Record authority/visibility history. New
commands refuse after Ledger adoption or abandonment; an existing authorized
receipt can still be recovered with endpoint details withheld. A deployment
policy must be fixed for the provider lifetime or derived entirely from the
captured Record. Live external policy mutation is not fenced by a Record append.

## Public browser contract

The authenticated API exposes:

- `GET /api/hale/v1/applications/{app}/dna/knowledge/commands/capability`
- `POST /api/hale/v1/applications/{app}/dna/knowledge/commands`
- `GET /api/hale/v1/applications/{app}/dna/knowledge/commands?request_id=...`

POST uses the normal same-origin command checks, JSON content type and
`X-Hale-Command: 1`. Its closed envelope is:

```json
{
  "request_id": "caller-generated-id",
  "operation": "dna.knowledge.edge.link",
  "operation_version": "1",
  "context": {"application_id": "<app>", "position_id": "org"},
  "target": {"application_id": "<app>", "kind": "dna.knowledge.node", "id": "<focused endpoint>"},
  "preconditions": {
    "principal": {"mode": "local", "name": "alice"},
    "record_head": "<checked Record head>"
  },
  "arguments": {
    "from_id": "sha256:<digest>",
    "to_id": "sha256:<digest>",
    "rel": "supports",
    "rationale": "Why this relationship belongs here."
  }
}
```

For removal, use `operation: "dna.knowledge.edge.unlink"` and add
`arguments.edge_id` from the selected graph relationship. All other fields have
the same shape. Capability discovery accepts optional
`?operation=dna.knowledge.edge.unlink`; omission retains the link capability.
Both operations share receipt lookup by request ID.

### Relationship Review

The same edge operations and arguments work under an explicit `review` grant.
Mode is chosen by the captured application policy; callers cannot select a mode
or override authority in the command. Direct command payloads, fingerprints,
facts and receipts retain their established encoding.

Reviewed admission appends `knowledge.edge.requested`, scoped to the original
command identity. Its canonical `dna.knowledge-edge-change/1` candidate pins the
ordered tuple, rationale, proposer, command fingerprint, grant and last Record
effect on that exact tuple. It is a separate Review candidate, not a Knowledge
node. Read its exact document through
`GET /api/hale/v1/applications/{app}/dna/reviews/candidate?id={review}&snapshot={head}`;
the typed response is `knowledge_edge_change`. Both endpoints and the candidate
must remain readable, including when inspecting a settled historical Review.

The supported native composition includes the owning Body, host/membrane delivery
and Review provider. The host and Body derive both endpoint author/target pairs
from their immutable receipts and require the same actual owner to admit all
four positions. Neither the caller's name nor the browser's viewing position
establishes ownership. A different owner's request is left for that owner;
cross-owner adjudication is not supplied by this profile. A standalone Knowledge
service can admit a request but cannot create its Review or consequence without
the Body. Capability availability describes admission support, not Body liveness.

The Body receives trusted requests, persists delivery, stores/readbacks the exact
candidate, creates its Review and acknowledges the proposal. Independent Review
decisions use the existing `dna.review.verdict@1` path. The actual effects are
`knowledge.edge.reviewed_linked` and `.reviewed_unlinked`, with separate declined
and apply-refused facts. Approval rechecks the exact tuple basis and endpoint
eligibility. Direct and reviewed link/unlink effects both advance that basis,
including absent-to-present-to-absent changes; a stale approved candidate is
refused at application rather than silently rebased. Unrelated tuples do not
invalidate it. Visible retired endpoints remain eligible for unlink.

Reviewed receipts add a closed `relationship` outcome object. Its presence
identifies this lifecycle even when details are hidden. Outer `recorded` means
request admission; `succeeded` means proposal/Review creation. Neither means
that the relationship exists. Effect state is independently `unknown`, `pending`,
`linked`, `unlinked`, `declined` or `refused`, and requires native evidence.
The top-level `edge_id` identifies the exact tuple even before its effect; it
does not claim graph presence. Protected candidate/endpoints clear identifying
details while retaining the reviewed receipt variant. Graph presence or complete
absence is checked separately through the existing authoritative read path.

Recovery follows the original admitted fact, regardless of later policy mode.
A reviewed request remains reviewed after switching to direct; an earlier direct
effect remains direct after switching to Review. Request keys are shared across
both paths and operations. The composed Body injects
`ops::StrictKnowledgeDirectEdgeFacts` as `Dna.edge_facts` to validate old direct
facts without duplicating their decoder. Its default adapter refuses direct
evidence it cannot validate. Current receipt visibility uses the existing strict
classification syntax injection; an already committed effect is recovered before
new-effect eligibility checks.

### Knowledge items and exact Review

The same endpoint also owns `dna.knowledge.node.propose@1`,
`dna.knowledge.node.revise@1`, and `dna.knowledge.node.retire@1`.
Creation takes `kind`, `name`, `text`, `author`, `target`, and
`rationale`. Its envelope target is `dna.knowledge.collection`, with the
same ID as the argument target. Revision adds `supersedes`; its envelope
target is the exact predecessor digest and kind `dna.knowledge.node`.
Retirement takes only `id` and `rationale`, targeting that exact node.
The service derives retirement name, author, target and predecessor from the
canonical retained receipt; the browser cannot substitute that metadata.

New or revised kinds are `idea`, `concept`, `practice`, and `task_concept`.
The predecessor must be visible, ratified and active. An adopted successor
already reserves it even if restart recovery has not yet written its retirement.
Existing unnamed items can be revised or retired; newly authored items have
a nonempty name.

Add any of these optional fields to an explicit principal grant:

```json
{
  "node_propose": "review",
  "node_revise": "review",
  "node_retire": "review",
  "node_scopes": [{"author": "org", "target": "org"}]
}
```

Each node operation is independently `review` or `deny`; omission denies it.
A node grant requires one to 32 exact author/target pairs. Lateral scope pairs
are invalid. Revision requires both the predecessor's pair and the candidate's
pair. These policy grants do not bypass native ownership routing or establish
the caller's authority to decide Review. Node capabilities in `review` mode
are available because this provider implements their native governance path.
There is no `direct` mode for these node operations.

One `knowledge.node.requested` fact binds the exact command, original
principal, policy basis and deterministic native candidate. The matching Body
receives it through the verified host/membrane route, records delivery, and
creates the native Knowledge proposal and Review. Recovery resumes only
delivered work; raw Record requests do not bypass host verification.
Use the composed native Review provider to decide the exact candidate through
`dna.review.verdict@1`. That provider checks canonical candidate evidence,
current authority, pending Review and independent reviewer eligibility.

Node receipts add a `node` object with proposal, Review and activation states.
The outer state is `recorded` while the proposal is pending, `succeeded`
when the proposal exists, `refused` when proposal admission was refused, or
`outcome_unknown` when the evidence is inconsistent. `succeeded` does not
mean adopted. Review approval and native adoption remain separate, including
approved candidates whose adoption is refused because another revision won.
The browser separately reads the graph to observe adoption or retirement.
Supersession preserves the predecessor receipt and historical bindings.

All Knowledge operations share durable request identity and recovery.
Node lookup suppresses target, candidate and Review IDs if the candidate or
predecessor becomes unreadable. It does not expose canonical body text,
rationale or native refusal prose through the outcome receipt.

Node limits are UTF-8 bytes: name, author and target 256; text 8192; rationale
2048; request ID 128; encoded HTTP request 98304. Text and rationale preserve
Unicode and non-NUL controls. Exact receipt documents are bounded at 98304
bytes so JSON escaping does not make an otherwise valid node unusable as a
relationship endpoint. Relationship request limits remain unchanged.

### Locus bindings and exact Review

`dna.knowledge.binding.bind@1` and `dna.knowledge.binding.unbind@1` change where an
existing item applies. Both take `idea_id`, `author`, `target` and `rationale`;
removal also requires the selected native `binding_id`. The envelope target is
`dna.knowledge.node` with ID equal to `idea_id`. Class and applicability are not
caller-controlled fields. The native class follows author/target ancestry;
lateral pairs are invalid. Binding identity retains the graph's exact digest of
the JSON array `[idea_id, target, author]`.

Add independent `binding_bind: "review"`, `binding_unbind: "review"` and
`binding_scopes: [{"author":"org","target":"org/support"}]` to a principal
grant. Each operation can instead be `deny`; omission denies. One to 32 exact
scope pairs are required for a binding grant. Node scopes, relationship grants
and viewing context confer no binding authority. Limits are 256 UTF-8 bytes per
locus, 2048 rationale bytes, and 32768 encoded request bytes.

Bind requires a visible, ratified, active item. Unbind can remove applicability
from visible retired items and can ensure an already-absent tuple stays absent.
Neither operation revises or retires the item. Other tuples, relationships,
receipts and historical task packages remain intact; future package selection
uses the resulting applicability.

The native candidate has format `dna.knowledge-binding-change/1`. It pins the
exact tuple, derived class, proposer, rationale, original command fingerprint,
captured grant and latest Record directive affecting this tuple. It is a separate
Review candidate, never a new Knowledge item. The exact authenticated read is
`GET /api/hale/v1/applications/{app}/dna/reviews/candidate?id={review}&snapshot={head}`.
It returns the canonical JSON document after checking the originating request
and current visibility of both candidate and underlying item. Existing node
candidate reads remain unchanged.

Native `knowledge.binding.bound` and `.unbound` facts record approved effects.
A changed tuple basis, including remove/re-add while Review is pending, produces
an explicit apply refusal. Approval remains visible separately. Recovery finds
an already committed effect before checking current eligibility. New effects
wait while the candidate or subject is protected. The composed Body supplies its
binding visibility syntax validator; without one, classified/filed receipts are
unavailable for new binding effects. Unclassified receipts remain eligible.

Binding receipts add only a `binding` outcome object: exact binding identity,
proposal and Review states, and effect state. `succeeded` means proposal/Review
creation. A terminal `bound`, `unbound`, `declined` or `refused` effect requires
its corresponding native evidence. Protected details are suppressed on recovery.
Graph observation is separate: presence needs the exact tuple; absence requires
the complete unfiltered binding sequence and a fresh visible receipt at the same
Record head. A working-locus filter cannot prove removal. Projection replays
original ratification, unbind and rebind in Record order, with exact-tuple no-ops.

The authenticated principal comes from the API; the payload only checks that
the caller's expected identity still matches. A relationship command's focused
target must be one of its endpoints. These operations use whole-organization acting context;
selecting a viewing position grants no authority.

The semantic precondition is the **exact Record head**, checked by atomic
append. A concurrent change requires a refreshed draft; the service does not
silently rebase. Endpoint membership, current visibility and authority are
resolved from that same captured Record. A graph snapshot/generation remains
read provenance and is not a claim of a transaction across Git and Postgres.

Request identity includes application, authenticated mode/name, the Knowledge
command namespace and request ID. It excludes operation. This namespace is
separate from the Practice/Review command endpoint. The canonical fingerprint
includes all typed command fields, including the expected Record head and exact
rationale. An identical retry recovers the existing fact before checking fresh
eligibility; changed content under the same identity conflicts.

The browser reserves recovery metadata before POST. An uncertain reply is
followed by GET lookup, not a blind second submission. The receipt retains the
original event, predecessor, sequence, fingerprint and captured authority. It
never echoes rationale or endpoint body text. Recovery checks current authority
and suppresses target/edge details when endpoint visibility is no longer known.

Limits are UTF-8 bytes: request ID 128; relationship label 256; rationale 2048;
HTTP request 32768. IDs and labels reject control characters. Rationale preserves
non-NUL controls and Unicode. Both endpoints use exact SHA-256 receipt identities.
New admission reads at most 10000 Record rows and bounded receipt documents.

Private `/graph/commands/{capability,submit,lookup}` routes accept authenticated
POSTs from the API with its resolved context. A read credential cannot use them.
The service owns admission and recovery; the API validates and presents their
results. This profile does not establish multi-clone ordering, routed Ledger
commands, cross-owner relationship adjudication, or hosted authentication
acceptance. Review-required operations never fall back to direct writes.

PostgreSQL service projectors built from these sources hold a session advisory
claim from projection begin through checkpoint. A competing projector refuses
that suffix until the owner completes or disconnects; a retry by the same session
does not acquire another lock reference. Read-only connections remain usable.
This ordering matters because unlink and later relink do not commute. All
projectors must use this protocol. Legacy processes and direct SQL writers that
ignore the claim are outside the supported command deployment. The local browser
acceptance uses a replayed memory graph; the optional two-connection PostgreSQL
gate must run before claiming the PostgreSQL deployment profile is accepted.
