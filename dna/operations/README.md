# Governance queries

`Queries` reads a caller-owned, already loaded `dna::Journal` and its receipt
provider. Construct it with `GitJournal` for a Record-only head. It never
refreshes, appends, publishes, starts a body, connects to Ledger or records a
receipt disclosure. The caller validates the source and owns the query lifetime;
`head()` and `revision()` identify that loaded view, not the current live tail.
A missing row returns an empty `id`; list order is first appearance in Record.
Repeated proposal/request facts do not duplicate the API catalog.

Practice identity is the exact canonical document digest. An unheard
`practice.requested` is a separate `PracticeRequestRow` and does not become a
practice with an invented digest. `state` projects activation facts independently
of `review_state` and `review_outcome`. Thus an approved Review can accompany a
pending or refused practice. Ratification survives a later decline, and retirement
has precedence. Locus author/target remain distinct from the human requester.

`text_available` describes the canonical practice receipt, and `text_status`
explains availability: `available`, `missing`, `redacted`, `protected`,
`source_unavailable`, `digest_mismatch` or `invalid_document`. Available bodies
must match their digest, their proposal metadata, and one of the two native
canonical codecs (`knowledge_document` or seeded `design_document`). A stored
blob alone never establishes permission. Record-only queries suppress body text
after any Ledger adoption because receipt restrictions may have lived in Ledger.
Abandonment does not restore those restrictions into Record and cannot reopen
body visibility. Missing, non-string or unknown classification cannot establish
public/internal visibility; a withheld marker always suppresses the body.

A knowledge Review also suppresses derived question/reason/refusal/diff/evidence
text and raw settlement when its practice receipt is unavailable. Its typed
`outcome` and structural state remain. This includes legacy Reviews linked only
by `subject_digest`. For a non-knowledge Review, `text_status=not_applicable`
means there is no canonical practice receipt; `text_available=false` says nothing
about its directly recorded question. After any Ledger adoption, even Reviews
without knowledge linkage suppress free text and raw settlement because their
subject visibility cannot be established. Public serializers still choose which
non-knowledge metadata they expose and must not expose arbitrary raw event bodies.

`review_row` and `Queries.practice_metadata` are the shared native projections for
legacy terminal consumers. They retain native text/provenance without public
receipt suppression; they are **not** the hosted body-read interface. The CLI
continues its existing receipt rendering in this slice. Hosted consumers use
`Queries.practice_of` / `review_of` (and their bounded page adapters), never those
raw helpers. This extraction does not claim to harden existing local CLI access.

Run `hale test dna/operations/tests` for real Git Record fixtures. They cover
separate review/activation state, pinned snapshots, source identity, pending
requests, preserved Unicode and multiline text, native design documents, malformed
receipts, protected/redacted stale blobs and adoption without a Ledger connection.
