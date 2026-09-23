# Plain Hale intake control

This application owns its SQLite database and declares one control operation,
`example.intake.set_mode@1`, through the face's application service,
`dna/face/service::ApplicationProvider`.
It uses no DNA Record, Ledger, positions, or administration messages.

`running` accepts one synthetic work item per committed 250 ms tick. `paused`
accepts none. Each tick records the configuration revision/value it actually
used, an observation counter, and a monotonic heartbeat. The work is this
example's counter increment, not an external job or delivery claim.

```
intake-control run DB OWNER [ticks]
intake-control serve DB PORT WEBROOT PRINCIPAL
intake-control info DB PRINCIPAL
```

Run the target and API as separate processes over the same file. Omitting ticks
(or 0) keeps the target running. Startup prints application_id and incarnation_id.
The API binds 127.0.0.1; its principal comes only from startup configuration. A
viewer may inspect, but only the stored owner may submit or recover commands.
The host account and database file permissions are this trusted-local example's
security boundary; this is not hosted authentication. Disable `LOTUS_OBS` for
ordinary tests; an observed example needs a private `/dev/shm`, because the
emitter sweeps stale observation segments at startup.

When observed, the application writes `DB.runtime.json` with its app/incarnation
and local process evidence. The provider checks the current SQLite incarnation,
fresh heartbeat and independently captured process key before exposing the
association. This optional sidecar is not command storage; stale, absent or
unreadable evidence omits the association. It never changes receipt recovery or authority.

A missing store is initialized once. Existing empty, incompatible, corrupt, or
wrong-owner stores refuse startup; they are never silently reset. The stable
application ID and owner persist. Every target startup generates a fresh random
incarnation, atomically fencing old workers. A bound provider refuses a recreated
store with another application ID. Heartbeat freshness is a two-second monotonic
observation window; it is not a durable command outcome. Effective state is reset
to unavailable on restart until the new target ticks.

Commands use prepared SQL, a 100 ms SQLite busy timeout, `BEGIN IMMEDIATE`, and
checked `COMMIT` with SQLite autocommit verification. Configuration and an
immutable receipt commit together with synchronous=FULL. A succeeded receipt
means the desired configuration committed; current effect is reported separately.
Identical retries recover before new revision/incarnation/offline checks. A
changed payload conflicts. Stale/offline new requests receive durable refusal
receipts. Current owner authority is checked before every lookup or retry. At 4096
receipts, new requests refuse; prior receipts remain recoverable without eviction.
Storage errors, including an uncertain commit, never report success: callers
must look up the existing request ID, not automatically retry POST.

All SQL identifiers/statements are fixed. Request fields are bound separately.
Each request/tick has a named SQLite connection and scoped statement owners;
failed paths finalize statements and roll back live transactions. The busy
policy does not bound filesystem I/O wall time. This example does not promise
operation across multiple machines or compensate for physical disk loss.

Build with the Hale compiler, the SQLite `sqlite3.h` development header and
a linkable `libsqlite3`. For a nonstandard development directory, configure
`C_INCLUDE_PATH` and `LIBRARY_PATH` before building. The SQLite engine is not
vendored. See `sqlite/PROVENANCE.md` for the Apache-2.0 Pond wrapper source and
local additions.

```
hale build dna/face/examples/intake-control/
hale build dna/face/examples/intake-control/tests/provider_test.hl
dna/face/examples/intake-control/tests/provider_test /absolute/scratch-directory
```

The focused proof covers real SQLite state, pause/resume effect, exact idempotency,
restart/fencing, authority, refusal recovery, failed writes and actual busy COMMIT,
identity/schema refusal, exact integer limits, and the 4096-row receipt boundary.
The capacity fixture inserts valid rows via the same prepared receipt writer;
it is not 4096 separate HTTP admissions. The browser/service integration exercises
the separate target and API processes.
