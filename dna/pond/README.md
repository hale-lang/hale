# dna/pond — a pinned copy of pond's Postgres driver

`db/` and `pq/` are `pond/db` and `pond/pq` from hale-lang/pond at
commit `01b8643` (2026-08-12), copied verbatim. The knowledge service
(`dna/knowledge`) speaks to Postgres through `db::DbDriver` and
`pq::PgConn`; the toolchain embeds these files beside the DNA core and
materializes them into its cache, so a governed project needs no
network and no `hale fetch` to run its knowledge service.

Refresh by copying the two directories from pond again and updating
the commit here; `hale check dna/pond/pq` must stay clean.
