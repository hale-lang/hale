# Memory and the Record

DNA persists all governance history and operational state in **PostgreSQL** inside a per-organism schema named `dna_<identity>`. Long-term governance decisions are stored in an append-only hash-chained table called the **record** (`record_rows`), while day-to-day execution state, organizational topology, and encrypted evidence live in **working memory** tables (`ledger`, `graph`, `claims`, and `receipts`). Access is isolated at the database layer using three PostgreSQL roles (`owner`, `spine`, and `head`); user devices never connect to PostgreSQL directly and instead query through [the Head](./head.md).

```sh
hale habitat migrate .          # start and migrate local Postgres, NATS, and Prometheus
hale dna record-rows            # dump signed governance rows (JSONL)
hale dna ledger rows            # dump operational workflow & task rows (JSONL)
hale dna show org               # query the projected organizational graph
```

## Storage Layers

An organism divides its state across three durable stores, with each row kind routed to a fixed table by `memory_of` in `dna/core/routing.hl`:

| Layer | Backing Store | Contents |
| --- | --- | --- |
| **Genome** | Git repository | Application source code, `dna/org/` configuration, candidate branches (`dna/candidate/<mutation>`), published deploy revisions (`dna/revision/<rev>`), and remote tracking ref (`refs/hale/remote/genome`) |
| **Record** | PostgreSQL (`dna_<identity>.record_rows`) | Signed, append-only audit log of governance and code transitions: mutations, Reviews, proposed/ratified practices, autonomy grants, cross-record connections, and enrolled people |
| **Ledger** | PostgreSQL (`dna_<identity>` tables) | Operational daily state: intents, tasks, human cases, workflow executions/steps/attempts, model call bills and receipts, budget reservations, schedules, concerns, readings, effect claims, and leases |

Every organism is provisioned with both `record_rows` and the `ledger` from its first row. `hale dna history` and the spine's projections read both tables as a single ordered sequence.

## The Record

### Schema, Hash Chain, and Identity

The record is stored in `dna_<identity>.record_rows`. Because all clones and host bodies read and write this PostgreSQL table directly, record rows do not travel over Git refs and require no Git synchronization.

Each row in `record_rows` contains:
- `seq`: Monotonically increasing sequence number assigned on insert.
- `kind`: Event type (e.g., `record.genesis`, `mutation.proposed`, `review.verdict`).
- `entity`: Target entity identifier (e.g., `m1`, `review:m1`).
- `body`: JSON payload.
- `author`: Logical identity of the person or component that authored the event.
- `writer`: PostgreSQL session role that inserted the row.
- `prev`: SHA-256 digest of the immediately preceding row (`"genesis"` for row 1).
- `digest`: `sha256(prev|kind|entity|body)`.
- `sig`: Cryptographic signature over the row.

`hale dna status` verifies the entire hash chain and prints `chain verified at <head>`. To export the raw record as newline-delimited JSON (`seq`, `kind`, `entity`, `body`, `author`), run:

```sh
hale dna record-rows
```

The record's `<identity>` is the SHA-256 digest of its first row, `record.genesis {genome, nonce}`, which combines the repository's root commit with a random nonce (`hale habitat graft` appends `record.genesis` followed by `record.key {key}`, the record's Ed25519 public key). Both the PostgreSQL schema and the NATS JetStream prefix are named `dna_<identity>`.

`hale habitat graft` pins this identity in `hale.toml`:

```toml
[dna]
record = "4f1c…"   # record genesis digest pinned at graft time
```

When you open a second clone or worktree, it locates its database schema from `[dna] record` and reads the `head` role DSN from `.hale/dna/memory.env` (populated by `hale habitat migrate` or `hale dna upgrade`).

> [!IMPORTANT]
> - **Copied manifests**: If `hale.toml` contains a `[dna] record` pin whose genesis commit does not exist in the local Git history, DNA rejects the checkout. Remove `[dna] record` from the copied `hale.toml` and run `hale habitat graft` again.
> - **Legacy Git-ref records**: Repositories containing `refs/dna/journal` from older toolchain versions are rejected by `graft` and `upgrade` (see [Troubleshooting](./troubleshooting.md)).
> - **Scratch directory (`.hale/dna/`)**: `.hale/dna/` holds only cached status projections, sandbox worktrees, and local runtime sockets. Deleting `.hale/dna/` never deletes record or ledger data.

### Insert-Only Appends & Optimistic Concurrency

Writers append to `record_rows` exclusively through two PostgreSQL functions: `record_append` (for a single row) and `record_append_rows` (for an atomic batch).
- Both functions acquire the schema's advisory lock and verify that the current tail digest matches the `prev` digest read by the caller.
- No database role holds `UPDATE`, `DELETE`, or `TRUNCATE` privileges on `record_rows`.
- If a concurrent writer wins a race, the losing caller reads the new tail digest and retries the append automatically.
- If a database read fails due to a transient connection drop, readers preserve their last known state and retry—a failed read is never misinterpreted as an empty or truncated record.

### Row Signatures & Trust Policies (`dna.trust`)

Every record row carries a signature in `sig`:
- **Runtime components** (`dna`, `spine`, `host`, fleet nodes, and legs) sign rows using the record's Ed25519 private key stored in the vault at `dna-exchange-<schema>`.
- **Human users** sign rows using their SSH signing key (`ssh-keygen -Y sign -n hale-dna-record`), resolved from Git's `user.signingkey` when `gpg.format` is set to `ssh`. If no SSH signing key is configured, the CLI writes an unsigned row under the name passed to `--as <who>`.

Every reader evaluates rows through `dna::row_admissible` according to the repository's `dna.trust` setting:

| `dna.trust` Mode | Verification Behavior |
| --- | --- |
| `local` (default) | Set automatically by `hale habitat graft`. Admits rows from any writer who has database access. |
| `signed` | Admits human-authored rows only when `sig` verifies against an SSH public key enrolled for that author in the record. Unverified request rows are rejected with `unverified writer` (never relayed to NATS) and skipped during graph projection. |

Manage enrolled SSH signers with `hale dna signers`:

```sh
hale dna signers enroll alice --key ~/.ssh/id_ed25519.pub
hale dna signers revoke alice
hale dna signers
```

Enrollment and revocation write `signer.enrolled {who, key}` and `signer.revoked {who}` rows to the record. Under `dna.trust = signed`, the first enrollment bootstraps the signer set, and every subsequent enrollment or revocation must be signed by an already-admitted signer.

### Candidate Branches & `hale dna sync`

Every candidate commit produced by a Mutation is preserved on a Git branch named `dna/candidate/<mutation>` so that rejected or revised changes remain inspectable after their sandbox worktree is cleaned up:

```sh
hale dna candidates                            # list kept candidate branches and Review states
hale dna candidates m7                         # show diff between m7 and its Review base
hale dna candidates drop m7 --why "superseded by m9"
```

`hale dna candidates drop` appends a `candidate.dropped` row and deletes the local `dna/candidate/m7` branch. Other clones delete their local copy of dropped candidate branches the next time they run `hale dna sync`:

```text
$ hale dna sync
record: the record is memory's: nothing to sync; 38 event(s)
```

## Receipts and Protected Evidence

Verification outputs, semantic diffs, model prompts, and user-filed documents are stored in PostgreSQL as content-addressed **receipts** referenced by SHA-256 digest. Every receipt carries a sensitivity class:

- **`public` and `internal`**: Stored unsealed in PostgreSQL by digest.
- **`customer` and `confidential` (Protected)**: Encrypted at rest in PostgreSQL. Only the digest and classification (`receipt.classified`) are written to `record_rows`. If working memory is unavailable, the payload is rejected and `receipt.withheld` is logged.

Protected receipts are encrypted using `HALE_DNA_RECEIPT_KEY` (minimum 16 characters), supplied in the owner's environment during migration and stored once in the owner-only `memory_keys` table. Supplying a different key on a subsequent migration is rejected so existing ciphertext remains decryptable. Runtime processes never read `memory_keys` directly; instead, they call three `SECURITY DEFINER` SQL functions owned by the schema owner:
- `receipt_file` and `receipt_read`: Callable by `head` and `spine` roles.
- `receipt_erase`: Callable only by `spine`; deletes the encrypted body while retaining the digest stub so the erased payload can never be re-filed.

> [!WARNING]
> A full PostgreSQL database dump (`pg_dump`) contains both the `memory_keys` table and the encrypted receipt bodies. Protect and retain database backups with the same access controls as the raw confidential evidence.

### Receipt CLI Commands

All `hale dna receipt` subcommands accept `--as <who>` to identify the caller:

| Command | Record Row Written | Description |
| --- | --- | --- |
| `hale dna receipt` | — | Lists all protected evidence receipts in the record |
| `hale dna receipt file <path> [--class internal\|customer\|confidential]` | `receipt.classified` | Stores a file as a content-addressed evidence receipt |
| `hale dna receipt disclose <digest> --to <who> --purpose <p>` | `receipt.disclosed` | Authorizes `<who>` to read `<digest>` for purpose `<p>` |
| `hale dna receipt show <digest> --purpose <p>` | `receipt.read` | Reads and prints a protected receipt after verifying a matching `receipt.disclosed` row |
| `hale dna receipt hold <digest> --why <w>` | `receipt.held` | Places a legal/audit hold preventing redaction until `release-hold` |
| `hale dna receipt redact <digest> --why <w> --policy <p>` | `receipt.redacted` | Erases the receipt body while preserving its digest stub |

Every protected read appends `receipt.read` before returning the plaintext. Every redaction appends `receipt.redacted` before erasing bytes (unsealed receipts are erased immediately; protected receipts are erased by the spine node on its next tick).

## Working Memory

Working memory is a direct PostgreSQL connection with role-based grants enforcing the trust boundary—there is no proxy service in front of the database. Each organism uses its own schema (`dna_<identity>`) and dedicated roles (`dna_<identity>_spine` and `dna_<identity>_head`), allowing a single PostgreSQL instance to host multiple organisms without cross-tenant access.

Role passwords are stored in the vault (`postgres-<role>`), and DSNs reference the vault entry via `&vault=postgres-<role>` rather than embedding plaintext passwords.

### Database Roles and DSN Environment Variables

| Environment Variable | Held By | Permitted Operations |
| --- | --- | --- |
| `HALE_DNA_MEMORY_DSN_OWNER` | `hale habitat migrate`, `hale dna memory migrate`, `hale dna upgrade`, `hale dna body provision` | DDL schema creation, role provisioning, and migrations |
| `HALE_DNA_MEMORY_DSN_SPINE` | Host supervisor (`hale dna dev` / `hale dna run`) and `dna/org` | Read/write record and ledger tables, project the graph, call `receipt_file` / `receipt_read` / `receipt_erase`; no DDL |
| `HALE_DNA_MEMORY_DSN_HEAD` | CLI commands, web face, and edge API (`hale dna head`) | Read tables, acquire/fence claims, append to the ledger via `ledger_append`, call `receipt_file` / `receipt_read` |

Run `hale habitat migrate` to migrate both PostgreSQL and NATS, or `hale dna memory migrate` to migrate PostgreSQL alone and print the role DSNs:

```text
$ hale dna memory migrate
HALE_DNA_MEMORY_DSN_SPINE=postgres://dna_…_spine@…/dna?sslmode=…&vault=postgres-dna_…_spine
HALE_DNA_MEMORY_DSN_HEAD=postgres://dna_…_head@…/dna?sslmode=…&vault=postgres-dna_…_head
```

Migration writes the `head` DSN to `.hale/dna/memory.env` (mode `0600`, Git-ignored) alongside `HALE_DNA_MEMORY_FROM` (recording whether the DSN points to the local habitat or a custom Postgres server). Subsequent `hale dna` CLI commands automatically load `.hale/dna/memory.env` unless overridden by `HALE_DNA_MEMORY_DSN_HEAD` in the environment. If `.hale/dna/memory.env` has overly permissive file permissions or points to an unrecognized origin, the CLI refuses to use it and prompts you to run `hale habitat migrate <dir>`.

### The Local Habitat

When `HALE_DNA_MEMORY_DSN_OWNER` is not set, DNA uses your machine's **local habitat** (`hale-habitat-local-<h>` under `${XDG_DATA_HOME:-~/.local/share}/hale/habitat/local`, requiring `docker compose` on `PATH`).

Each organism is provisioned as a set of three Docker Compose services—`memory-<id>`, `nerves-<id>`, and `senses-<id>`—where `<id>` is the first 12 hex characters of `[dna] record`. Their generated `compose.yaml`, `ports.yaml`, and mode `0600` secret files live in `<habitat>/allocations/<id>/`. To inspect the containers directly:

```sh
docker compose -p <project> --project-directory <habitat>/allocations/<id> -f compose.yaml -f ports.yaml ps
```

Key operational rules for the local habitat:
- **Explicit migration only**: `hale habitat up`, `hale habitat graft`, and `hale habitat migrate [organism]` are the only commands that start containers or apply schema migrations. Each migration runs in a single transaction and stamps the schema version; if a host connects to a mismatched schema version, it exits and instructs you to run `hale habitat migrate <organism>`.
- **No implicit migration in `dev` or `run`**: `hale dna dev` derives the `spine` DSN from the running habitat allocation and passes only that DSN to the host. `hale dna run` requires `HALE_DNA_MEMORY_DSN_SPINE` in its environment, strips any `owner` or `head` DSNs, and refuses to start if the spine DSN is missing.
- **Habitat lifecycle commands**:
  - `hale habitat up`: Births the habitat organism on first run, starts all recorded allocations, and runs the habitat's `body` and `face` as Docker Compose services.
  - `hale habitat status`: Lists every organism allocation, role, and migrated schema version, highlighting any allocations behind the current toolchain.
  - `hale habitat down`: Stops all habitat containers while preserving their volumes and data.

### Shared Multi-Owner Records

When multiple organizations share a single record, each organization's heads authenticate with a dedicated per-owner PostgreSQL role (`dna_<identity>_head_<owner>`). Pass `HALE_DNA_OWNER_KEYS="<owner>=<key> …"` during migration to provision per-owner roles and print `HALE_DNA_MEMORY_DSN_HEAD_<OWNER>=<dsn>`. Each owner's head processes use their respective DSN as `HALE_DNA_MEMORY_DSN_HEAD` and may only author rows on behalf of members belonging to that owner in the organizational graph.

## The Ledger

The operational ledger tracks daily workflow execution state and is created alongside `record_rows` during initial migration. To dump all ledger rows as newline-delimited JSON:

```sh
hale dna ledger rows
```

When you run a command that records operational activity (such as `hale dna task done`), the CLI connects under the `head` role and calls the `ledger_append` PostgreSQL function directly. The database gate validates and commits the row immediately—or rejects it synchronously with an error:

```text
$ hale dna task done acme:t4 --as alice
hale dna: `task.done acme:t4` was not written to the ledger: task acme:t4 is handed to bob, not to alice; a completion is admitted in the assignee's name
```

`ledger_append` rejects a write when any of the following conditions hold:
1. The row kind belongs to `record_rows` rather than the ledger.
2. The named author has been retired (`person.retired`).
3. The calling PostgreSQL role is not authorized to write on behalf of that author.
4. A task transfer crosses outside the organization that owns the task.
5. `task.done` is submitted by anyone other than the current assignee.
6. The write references an expired or superseded lease epoch (`fenced`).
7. The write was computed against an outdated ledger revision (`stale_revision`) or claims an already-claimed entity.

Every ledger write includes a client request ID so retries after a network drop are idempotent. If PostgreSQL becomes unreachable, `hale dna status` displays `memory: THE LEDGER IS UNREACHABLE (…)` and halts new admissions until connectivity returns.

## The Graph

On every 1-second tick, each spine node projects new `record_rows` entries into PostgreSQL graph tables: ratified knowledge and bindings, observed code structure, raised concerns and pressure counts, repository nodes and edges (`graph.node`, `graph.edge`), and retired personnel.

- **Compare-and-swap projection**: Each projected row updates the graph and advances the projection watermark in a single transaction guarded by compare-and-swap, allowing multiple spine nodes to project concurrently without duplicating effects.
- **Malformed row isolation (`graph.refused`)**: If a record row contains an invalid graph mutation (e.g., referencing nodes of the wrong kind), memory appends a `graph.refused` row (`by: memory`) naming the offending row and reason, then advances past it so a bad proposal never blocks subsequent graph reads.

The organizational chart is stored directly in this graph via `holds` edges. Query the two primary views with:

```sh
hale dna show org               # positions, parent hierarchy, holders, and signing authority
hale dna show processes         # processes, contract boundaries, and runtime placement
```

Both commands require `HALE_DNA_MEMORY_DSN_HEAD` and support `--json`. If graph projection lags behind `record_rows`, they print `memory has projected N of the record's M rows` on stderr before rendering.

### Re-Ingesting Repository Structure (`hale dna ingest`)

While `hale habitat graft` inspects the repository graph once at creation, `hale dna ingest [--at <rev>]` re-analyzes the repository at a given commit (`HEAD` by default) and opens a single Board Review (under group `ingest`) containing any added nodes/edges, modified bodies, and retired nodes:

```text
ingested 3f2a…c1: 6 added, 0 changed, 3 retired; the Board ratifies it as k:… (`hale dna review` lists it under `ingest`)
```

Every operation exposed by an API surface in the repository becomes a `tool` node (`tool:<Surface>::<op>`) recording its contract and required roles. Each run writes `graph.ingested`; once ratified, re-running `hale dna ingest` on the same commit reports `the graph the record states is the repository's; nothing to review`.

### Context Targets & Toolchain Library Families

Knowledge ideas and mandates are bound to **targets** in the graph:
- **Locus path target** (`org`, `org/<child>`): Matches the path itself and all descendant paths beneath it.
- **Node ID target** (`language:hale`, `system:dna`, `position:leader`): Matches only that exact node ID (never prefix-matched).

When a worker claims a unit of work, its context **hat** queries PostgreSQL for all ratified ideas matching the work's target set:
1. The work's locus path.
2. The `written_in` language nodes of the attached application (or `system:dna` when modifying `dna/org/`).
3. The performer's position node (including its ratified mandate).

These resolved targets are recorded in the hat's JSON (`targets`) and included in its content digest. `hale dna review <id>` queries the same target set to display relevant ratified knowledge under `knowledge for …`.

At graft time, DNA creates the root `language:hale` and `system:dna` nodes (plus `application:<name>` linked via `written_in` to `language:hale`) and proposes the built-in toolchain documentation as two **library families**:
- `library/language@<version>`: Book chapters and spec sections bound to `language:hale`.
- `library/design@<version>`: DNA architecture chapters and specs bound to `system:dna`.

Ratify both families in one command so workers receive ranked, token-budgeted documentation chapters matching their task:

```sh
hale dna review                      # lists library families alongside practice groups
hale dna review library approve      # ratifies both families and binds all chapters
hale dna review <family-id>          # inspects the individual chapters in a family
hale habitat graft <name> --no-library    # provisions a record without library proposals
```

When you update the toolchain and run `hale dna upgrade`, DNA proposes new `@<version>` library families; approving them atomically replaces and retires the previous version's bindings.

## Cross-Record Connections & Handoffs

Organizations with separate records can exchange tasks and evidence receipts over NATS using mutually ratified **connections**:

```sh
hale dna connect nats://acme.example:4222 --record 4f1c… --name acme \
    --as accountant --purpose "year-end books" --classes internal,customer
hale dna handoff acme task t7
```

1. **Identity cards & connection ratification (`connect`)**: `hale dna nerves migrate` publishes a signed identity card on the organization's NATS server containing its `[dna] record` identity, genome digest, and Ed25519 public key. `hale dna connect` fetches the peer's card, pins its public key, and opens a Board Review. The connection becomes active once approved by a Board member other than the proposer. Run `hale dna connect` with no arguments to list active connections.
2. **Inbox credentials**: To deliver envelopes to a peer's NATS server, store the peer's `exchange` NATS password in your vault as `nats-<peer-org>-exchange`.
3. **Delivering tasks and receipts (`handoff`)**: `hale dna handoff <name> task <id> | receipt <digest>` signs an envelope with your record's Ed25519 key and publishes it to the peer's bounded inbox. Only human-assigned (`handed`) tasks may cross records. Receipts cross as `(digest, class)` metadata only (never raw bodies) and only if the connection permits that sensitivity class.
4. **Mutual admission (`handoff accept` & `handoff sync`)**: The receiving organization verifies the envelope against its own reverse connection pinning your key, then accepts the work with `hale dna handoff accept <id>`. Running `hale dna handoff sync` on the sending side reads the acceptance receipt and settles the local task.
5. **Disconnecting (`disconnect`)**: Run `hale dna disconnect acme --why "the engagement ended"` to revoke future handoffs while preserving all previously exchanged records.
