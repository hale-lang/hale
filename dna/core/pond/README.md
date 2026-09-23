# dna/core/pond — a pinned copy of pond's Postgres driver

`db/` and `pq/` are `pond/db` and `pond/pq` from hale-lang/pond at
commit `01b8643` (2026-08-12), copied verbatim. The core opens memory
through `db::DbDriver` and `pq::PgConn` (GH #985); the toolchain embeds
these files with the core and vendors them beside it (`vendor/dna/pond`),
so a governed project needs no network and no `hale fetch` to reach its
memory.

Refresh by copying the two directories from pond again and updating
the commit here; `hale check dna/core/pond/pq` must stay clean.
