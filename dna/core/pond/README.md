# dna/core/pond — pinned copies of pond libraries

Copied from hale-lang/pond, verbatim but for what is named below. The toolchain embeds these files
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
  Two changes of this repository's are not in pond yet: `NatsConn`'s
  `untyped` prefix, whose messages go out on the local `NatsInbound`
  topic as they arrived, for the host's reading of an application's
  events (GH #987); and `NatsPublisher`, a publish-only adapter that
  dispatches nothing into its program, so an application that says a
  concern keeps an exact artifact its fleet's law can be certified over
  (GH #986). A refresh keeps both until pond has them.

Refresh by copying the directories from pond again and updating the
commits here; `hale check dna/core/pond/pq` and
`hale check dna/core/pond/realtime/nats` must stay clean.
