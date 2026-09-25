# dna/core/pond — pinned copies of pond libraries

Copied verbatim from hale-lang/pond. The toolchain embeds these files
with the core and vendors them beside it (`vendor/dna/pond`), so a
governed project needs no network and no `hale fetch` to reach its
memory or its nerves.

- `db/`, `pq/`: `pond/db` and `pond/pq` at `01b8643` (2026-08-12). The
  core opens memory through `db::DbDriver` and `pq::PgConn` (GH #985).
- `realtime/nats/`: `pond/realtime/nats` at `ef90234` (2026-09-24), its
  source files only (not its tests or examples). The nerves (GH #986):
  the node's host and the organization bind their topics to its
  `NatsAdapter`, and a pinned `NatsConn` carries them over NATS
  JetStream.

Refresh by copying the directories from pond again and updating the
commits here; `hale check dna/core/pond/pq` and
`hale check dna/core/pond/realtime/nats` must stay clean.
