# Knowledge projection and cockpit reads

`KnowledgeStore` is the service-owned projection of native knowledge decisions.
`Mem` and `Pq` retain native idea names and exact supersession digests on insert,
replacement and retirement. The Record tail now carries both fields from the
canonical document. `Idea.provenance` keeps its existing lifecycle meaning
(`proposed`, `ratified`, `declined`, `retired`); it is not relabeled as the source
document's provenance.

Postgres adds `name` and `supersedes` with idempotent migrations. Existing rows
receive empty values. This migration does not reconstruct metadata previously
discarded by the tail: an explicit projection rebuild/backfill is still needed
for that history. The cockpit must not invent missing lineage. Memory binding
replacement now matches Postgres: the same `(idea, target, author)` triple updates
its class without adding another binding or changing applicability.

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

This prerequisite is not a public Knowledge graph endpoint. Bounded store
enumeration, coherent graph generations, service authentication, filtered graph
responses and browser integration remain required. No graph capability is
enabled by this change.

## Verification

- `dna/tests/knowledge_projection_fidelity_test.hl`: native document/tail/store
  metadata, replacement and retirement, binding parity; optional Postgres legacy
  schema migration and persistence through a new connection.
- `dna/tests/knowledge_visibility_test.hl`: routed restrictions, malformed
  policies, inaccessible Record, adoption/abandonment, stale snapshots and limits.
- `dna/tests/ledger_read_health_test.hl`: recovery after a command conflict,
  unopened reads; optional Postgres gaps, matching-count corruption and recovery.
- The existing `dna/tests/ledger_test.hl` checks established Ledger behavior.

Postgres tests use `HALE_DNA_KNOWLEDGE_DSN` and private per-run schemas. A missing
DSN is an explicit skip, not Postgres validation. Compile and run native tests
separately with hard memory/CPU limits; a wall timeout alone does not contain
native allocation failures.
