# Knowledge projection and face reads

`KnowledgeStore` is the service-owned projection of native knowledge decisions.
`Mem` and `Pq` retain native idea names and exact supersession digests on insert,
replacement and retirement. The Record tail now carries both fields from the
canonical document. `Idea.provenance` keeps its existing lifecycle meaning
(`proposed`, `ratified`, `declined`, `retired`); it is not relabeled as the source
document's provenance.

Postgres adds `name` and `supersedes` with idempotent migrations. Existing rows
receive empty values. This migration does not reconstruct metadata previously
discarded by the tail: an explicit projection rebuild/backfill is still needed
for that history. The face must not invent missing lineage. Memory binding
replacement now matches Postgres: the same `(idea, target, author)` triple updates
its class without adding another binding or changing applicability.
An identical class is a row-level no-op. `unbind` removes only that exact triple;
removing an absent tuple is also a row-level no-op. Reviewed binding effects are
replayed from the Record in order, including removal and later re-addition.
Existing task packages and their historical evidence remain unchanged. Native
write profiles are documented in [service/COMMANDS.md](service/COMMANDS.md).

Relationship commands can use direct or reviewed admission according to the
service's explicit policy. Reviewed changes preserve the existing directed tuple
identity and command encoding. Their candidate pins the exact endpoints, label,
original grant and latest tuple directive; it is separate from Knowledge item
content. The native Body records the Review and a distinct linked/unlinked,
declined or refused consequence. The projector validates that consequence and
its historical tuple basis before applying it. A changed basis cannot be hidden
by an intervening unlink/relink, and reverse or differently labeled tuples stay
independent. The composed Body injects `StrictKnowledgeDirectEdgeFacts` so direct
and reviewed directives share the same validated history. See the command guide
for the same-owner reference profile and its recovery limits.

## Visibility prerequisite

`KnowledgeVisibility` in `tail.hl` captures receipt restrictions from Record and,
after adoption, the matching Ledger. It is a per-request child, with a combined
10,000-row / 8 MiB scan budget by default. It accepts canonical and bare SHA-256
receipt IDs and excludes protected, withheld, redacted and unknown-class nodes.
Restrictions remain effective even if a later event labels the receipt public.
It does not fetch protected bodies or create disclosure receipts.

The service composition must supply a strict `KnowledgePolicySyntax`; the
existing operations `OrganizationJson` satisfies it. Missing validation refuses
interpreted policy documents. Malformed JSON, duplicate/escaped keys and invalid
policy field types cannot grant visibility. An empty/genesis Record cannot
authorize cached graph content because the legacy Journal interface also uses
that representation after some read failures.

The service must scope both memories to the same Record, establish identity and
authority separately, capture `load()`, filter nodes **before** pagination,
labels, edges, bindings or counts, and call `unchanged()` immediately before
publishing. Both endpoints of an edge must be visible. A failed or stale capture
returns an explicit unavailable/stale result, never a successful empty graph.
The basis preserves separate Record and Ledger heads and revisions; it does not
claim a transaction across them. Abandoned Ledger history remains unavailable
because abandonment can erase restrictions that never returned to Record.

Ledger refresh now reports the current read's failure instead of retaining an
old command conflict. It rejects unopened sources, sequence gaps, shrinking
history, changed tails at the same revision and incomplete/concurrently changed
loads. Postgres checks count, sequence extent and tail in one statement. This
does not turn the Ledger into a graph transaction or replace source identity.

## Authenticated graph reads

The source-built public DNA API can read this store through the private native
service's `POST /graph/read`. Configure `HALE_DNA_KNOWLEDGE_READ_KEY` on both
services (at least 32 bytes, no control characters), and
`HALE_DNA_KNOWLEDGE_URL` on the public API. The browser never receives this key.
The public head authenticates its reader before making the private request;
the state service verifies the credential before source reads or store access.
See [the API contract](../api/README.md#knowledge-reads) for public routes.

`GraphStore` is a separate structural read interface. Its native node, edge and
binding pages have bytewise tuple order and visible-only cursors. Filtering
precedes page size and `has_more`; no global total or hidden cursor key is
exposed. Both endpoints must be visible for an edge. A protected predecessor
identity is omitted from `supersedes` as an empty string, which means unavailable
in this view rather than proof that no predecessor exists. An optional target
filters node/binding relevance, not authority; incident edges retain the same
view basis without claiming target applicability.

Postgres captures each page and its metadata in a read-only repeatable-read
transaction. Transactional triggers track graph changes, including direct SQL
updates, deletes and truncates. An indexed identity-only lookup establishes the
requested node's existence without fetching its body for relationship reads.
The Mem implementation uses the same typed rows and generation checks. It scans
its bounded in-memory source, so it can refuse a large source for which indexed
Postgres can return a small page. Postgres fetches one candidate per round trip
to avoid the current native driver's multirow buffering costs.

Projection proof includes a persisted applied Record head and revision, separate
from the numeric tail watermark. Before applying more rows, the service records
the exact intended suffix as a pending projection using compare-and-swap. Only a
fully successful application checkpoints that suffix and clears the pending
marker. A failed event may have written a node before its binding or watermark;
those partial writes must never be mistaken for a clean projection. Restart may
resume only when both the applied prefix and pending target are exact prefixes
of the current Record. Rewinding or replacing the pending suffix makes the
projection unavailable until the original source is restored or an explicit
rebuild is performed. The marker is a recovery fence, not an atomic transaction
over all event effects.

An older projection without this proof is unavailable until an explicit
migration/rebuild; current source identity is never assigned retroactively.
Reads require a complete, non-pending Record projection, matching scope and
unchanged graph generation and Record/Ledger visibility before publication.

Pages are bounded to 100 visible rows and a 1 MiB response. Store scans have a
10,000-row/8 MiB budget; memory identity probes share that budget. Projection
catch-up has its own 8 MiB receipt budget and checks pinned, direct Git blob types
and sizes before fetching bodies, then verifies their exact size and digest.
Record row/byte budgets are checked before admission scans and projection.
These checks do not make the existing eager GitJournal constructor or native
JSON implementation allocation-safe; the staged stdlib buffer repair remains a
delivery prerequisite for large inputs.

The graph read routes are read-only. They do not establish original receipt provenance,
backfill missing historical metadata, infer run/definition/practice consumers,
implement curation, or grant position authority. Unavailable provenance and
dependency coverage are explicit in the wire format. A protected receipt whose
canonical body is absent can still make the existing projection unavailable;
this route does not copy protected bodies into a public projection to hide that
condition. Deployment deadlines and packaged-source registration remain tracked
in the service development plan.

## Verification

- `dna/tests/knowledge_projection_fidelity/cases.hl`: native document/tail/store
  metadata, replacement and retirement, binding parity; optional Postgres legacy
  schema migration and persistence through a new connection.
- `dna/tests/knowledge_visibility/cases.hl`: routed restrictions, malformed
  policies, inaccessible Record, adoption/abandonment, stale snapshots and limits.
- `dna/tests/ledger_read_health/cases.hl`: recovery after a command conflict,
  unopened reads; optional Postgres gaps, matching-count corruption and recovery.
- `dna/tests/knowledge_graph_store/cases.hl`: bounded visible pagination, tuple
  ordering, generation tracking and persisted projection checkpoints in memory
  and PostgreSQL.
- `dna/tests/knowledge_graph_service/cases.hl`: authenticated native reads over
  actual Git Records/receipts, projection lineage, private-body filtering and
  concurrent Record/Ledger/store changes. This suite runs in an isolated child.
- The registered `dna/tests/knowledge_store_test.hl` explicitly runs the fidelity
  and visibility cases and both graph suites; `dna/tests/ledger_test.hl` runs
  read health alongside the established Ledger behavior. The parent retains the
  PostgreSQL DSN while the service child clears private settings. All participate
  in the existing native CI suite without changing its fixture inventory.

Postgres tests use `HALE_DNA_KNOWLEDGE_DSN` as the schema owner's DSN and private
per-run schemas: `know::migrated(owner, record)` applies the schema and hands back
the record's spine DSN, which the stores open with (GH #985); cleanup drops the
schema and the role as the owner. A missing DSN is an explicit skip, not Postgres
validation. Compile and run native tests
separately with hard memory/CPU limits; a wall timeout alone does not contain
native allocation failures.
