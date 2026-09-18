# Iris cockpit readiness

Source assessment, updated 2026-09-18. This records implementation readiness for
the four core cockpit workspaces; it is not a full runtime acceptance audit.
The subsequent [service development plan](../dna/SERVICE-DEVELOPMENT-PLAN.md)
extends this audit with deployment and command-recovery findings, implementation
cards and the results of two targeted native tests.

The original assessment distinguished two revisions:

- **Main baseline:** [`2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3`][main].
- **Pending workflow work:** [`cd8dcc43de744f28ec5f93b1a900f166bed2defd`][pending].
  This supplied the initial definition-model assessment.

The September 18 integration uses main `9a3136ca`: workflow cards 00–06 and
request-relay fix #692 have merged. Definitions, durable facts and pure projection
are available, but that main revision has no workflow admission or executor
wiring. Admission #696 was still open at this update and itself dispatches no
work. These facts supersede the original pending status.

The [read API](../dna/api/README.md) and [browser cockpit](cockpit/README.md)
implement the practice/review read slice, including exact identities, source
revisions, authenticated reads, unavailable content and snapshot-bound paging.
The browser serves beside the API and links to the independent Runtime observer.
It advertises no mutations or position context.

The branch also supplies a compiler-backed Organization inspector: committed
source, captured dependency bytes, explicit declaration groups, exact static
containment and typed contracts. Its ownership map remains separate from
compiler instance identities. This is a declared-structure read foundation,
not a semantic position/performer catalog or an acting-context contract.

## Four-workspace matrix

| Workspace | Current foundation | Browser/API gap | Next dependency |
|---|---|---|---|
| **Organization** | Positions are loci, routing is the bus, and capabilities are contracts. The branch projects source-declared static instances/contracts and separate ownership maps through a typed read API and browser inspector. | Semantic position-to-instance/owner/performer bindings, purpose/responsibility metadata, effective-position capabilities and governed edits remain absent. Static declarations do not prove occupancy. | Add source-declared semantic bindings and context, then source-backed editing; preserve person, position, owner and authority as distinct relationships. |
| **Knowledge** | Ideas, typed edges, bindings, provenance, proposal/ratification, retirement and bounded position-relative context. The service owns a persistent projection. | `/context`, `/idea/:id`, `/structure` counts/names and `/signals` are useful reads, not a complete graph API. No neighborhood/traversal/list or general graph-edit routes; the store interface also lacks edge enumeration. | No relevant change in the inspected pending revision. Graph browsing and administration need service/query additions, not only frontend rendering. |
| **Practices** | Named proposals with author/rationale, exact-digest Board review, ratification and supersession; typed list/detail API and live browser reads. Historical content remains in receipts. | No proposal/verdict HTTP command contract. The proposal entry binds to **`org`**, without arbitrary position scope. The read adapter withholds receipt bodies after Ledger adoption until authoritative visibility can be established. | Strongest first administrative slice: durable command identity/recovery, exact-subject decisions and authoritative content visibility. |
| **Definitions** | Merged `WorkflowCatalog` supplies code-authored versioned ordered steps, leaf/child members, validation, bound expansion and JSON round-trip. Facts and pure execution projection have merged too. | No running-app catalog export, browse/edit/publish API or canonical catalog administration lifecycle. The browser must not invent an authoritative workflow model. | Add an application-declared catalog capability. Keep current definitions distinct from immutable execution recipes; read execution facts from their routed Record/Ledger source. Admission and executor integration remain separate. |

Evidence: [organization semantics][org], [ownership data and approval][ownership],
[supplied review policy][policy], [knowledge schema][knowledge],
[knowledge reads][knowledge-http], [store interface][store],
[practice command][practice-cli], [practice scope][practice-scope],
and the [merged definition schema/catalog][definitions].

## Current head and authority boundary

The [legacy HTTP head][head] exposes status, Board, fleet, reviews, history,
verdict, ask and pressure. It does not expose the four model workspaces as
structured query/edit resources. OIDC mode resolves a session to a person and
Board/reviewer authority; local mode retains its local-trust behavior. Neither
mode supplies the proposed active-position session and capability contract.
The new read API consumes shared typed operations instead of dispatching those
legacy CLI-backed routes. Its capabilities describe only the supported reads.

Knowledge `target` selects relevant content; it does not prove the caller may
act as that position. The service's context/idea routes do not establish a human
session boundary. Shared-ledger writes have separate [owner-key/membership and
fencing checks][service-authority]. Compose these behind an authenticated head;
do not expose the internal service as if it were already a scoped cockpit API.

The existing verdict endpoint does not forward the digest the browser displayed.
The new interface must submit the exact subject and relevant preconditions,
rather than resolving an unspecified current subject when the operator clicks.
The domain Review remains responsible for admission. [OIDC tests][oidc-tests]
already check session attribution and refusal without a session; they do not
establish the new position-context or model-edit API.

Two source-level issues reinforce why the adapter needs a typed contract:
the current ask endpoint passes `to` through a [sanitizer that strips slashes][head-clean],
so a position path loses its identity, and the [CLI wrapper/text response][head-text]
does not preserve failed command exit status as an HTTP error. These findings
were source-traced, not reproduced in a new runtime test here.

## First vertical slice: revise a practice and observe its use

Deliver this complete path before expanding individual screens:

1. Browse the organization's active and proposed practices, with full content,
   provenance, governing review and predecessor/successor links.
2. Draft a revision against a named active digest and submit it in the signed-in
   person's identity, with rationale and a stable request identity.
3. Inspect and decide the exact proposed document under effective authority.
4. Follow proposal, refusal or ratification, and supersession to authoritative
   recorded results; recover the same request after reconnect.
5. Read a position's resulting context package and link its included practice
   back to that version. The old document and decision remain inspectable.

This exercises Practices, Knowledge and Organization context without depending
on the unfinished generic workflow execution path. It is a first slice, not
completion of the other core workspaces and not a reason to defer them.

The current command is organization-wide and Board-ratified. Preserve that
behavior until the platform has an explicit position-scoped proposal/admission
contract. Do not present a scope selector as a capability the backend lacks.
Practice text is guidance unless a specific mechanism enforces it; ratification
does not by itself make arbitrary prose a compiler rule.

The first API surface is bounded: typed practice projections, authenticated command
submission, a request/result lookup, exact-subject verdicts, and context links.
The CLI currently [mints a time-based request ID][practice-cli]; rerunning the
command after a lost HTTP response is not request recovery. Reuse the durable
proposal/relay semantics while making identity and reconciliation explicit.
This also requires domain work: concurrent request admission, interrupted
proposal/ratification recovery and per-command verdict correlation. Review approval
and successful practice adoption are separate results. The service plan makes
those gaps explicit; a typed HTTP wrapper alone does not close them.

Acceptance must cover inadequate authority, wrong subject, duplicate delivery,
reconnect and competing supersessions. The existing [knowledge tests][review-tests]
exercise wrong-digest and wrong-authority refusal; [practice tests][practice-tests]
exercise proposal, attribution and pending state. The [supersession guard][supersession]
refuses ratification against an already-retired predecessor. These are foundations
to reuse, not evidence that the future browser slice has passed.

## Dependency boundaries and subsequent stages

- **Frontend:** navigation, active-position context, graphs/outlines, forms,
  drafts and impact presentation. It owns no alternate organization, knowledge
  authority, definition registry or workflow scheduler.
- **Head/API:** authenticated structured queries, permitted viewing/acting
  contexts, stable identities/versions, command admission forwarding, correlated
  receipts, freshness and reconnect. Preserve backend access/redaction when
  composing data; hiding a field in JavaScript is not admission control.
- **Domain services:** canonical state, authority, provenance, ratification,
  source-backed change, catalog lifecycle, adoption and execution. Missing domain
  operations must be named as dependencies rather than emulated in the UI.
- **Observer:** runtime evidence with application/artifact identity and coverage.
  Ordinary Hale observation remains useful without DNA; business completion is
  not inferred from process or locus disappearance.

Next, expose graph neighborhoods and binding provenance; add governed knowledge
edits and position-scoped practice administration where supported. In parallel,
project the actual organization and provide source-backed edits with impact and
review. The merged definition schema can support catalog browsing, structured
drafting and validation now, but publish/activate/run must wait for authoritative
catalog lifecycle and execution adoption. Its [revision and serialization tests][definition-tests]
are useful integration contracts, not main-branch browser features.

Finally join admitted definitions to durable runs and runtime evidence, test the
full four-workspace authoring-to-outcome loop, and harden history, concurrency and
reconnect across it. General drag-and-drop Hale assembly is later; core organization,
knowledge, practice and task/workflow administration is not that later milestone.

[main]: https://github.com/hale-lang/hale/commit/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3
[pending]: https://github.com/hale-lang/hale/commit/cd8dcc43de744f28ec5f93b1a900f166bed2defd
[org]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/core/org.hl#L1
[ownership]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/core/ownership.hl#L200
[policy]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/core/assembly.hl#L77
[knowledge]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/core/knowledge.hl#L15
[knowledge-http]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/knowledge/service/main.hl#L516
[store]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/knowledge/store.hl#L44
[practice-cli]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/host/writers.hl#L631
[practice-scope]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/core/assembly.hl#L1490
[definitions]: https://github.com/hale-lang/hale/blob/9a3136ca801d1f84282614b9eb5529554f94308b/dna/core/workflow_definition.hl#L309
[head]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/ui/main.hl#L229
[head-clean]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/ui/main.hl#L60
[head-text]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/ui/main.hl#L34
[service-authority]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/knowledge/service/main.hl#L194
[oidc-tests]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/tests/principal_oidc_test.hl#L193
[review-tests]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/tests/knowledge_events_test.hl#L77
[practice-tests]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/tests/practice_test.hl#L49
[supersession]: https://github.com/hale-lang/hale/blob/2f202c9094fff5f4219bd9ede6a1af0a4ba27ae3/dna/core/assembly.hl#L652
[definition-tests]: https://github.com/hale-lang/hale/blob/9a3136ca801d1f84282614b9eb5529554f94308b/dna/tests/workflow_definition_test.hl#L121
