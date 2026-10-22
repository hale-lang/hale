# Operational Troubleshooting Guide

This guide indexes exact diagnostic messages printed by `hale dna`, `hale habitat`, and `hale node`, grouped by subsystem, along with their root cause and remediation steps.

- One-shot CLI verbs print errors prefixed with `hale dna: <reason>`.
- Running body processes prefix log lines with `hale dna run: ` or `hale dna dev: `, and fleet node agents prefix lines with `hale node <name>: `.

Four read-only commands work offline (even when no organism body is running) and are the primary starting point for diagnosing state:

```sh
hale dna status            # status projection: organism lease, journal chain, reviews, body, memory
hale dna board             # items awaiting Board action, missing vault secrets, and recent violations
hale dna history t1        # causal event chain for a specific Task, Work, attempt, or Review
hale dna secrets           # required vault secret slots and whether each is populated
```

---

## The Record

### Nothing is running here

```text
organism:   not running — reading the Journal
```

- **Cause:** `hale dna status` prints this when no body process currently holds the record's lease. Read-only commands (`status`, `history`, `board`, `review <id>`, `ui`) still function normally by reading the Postgres record directly.
- **When submitting tasks:** If you run a command that waits for the organism to respond (such as `hale dna task create`) while no body holds the record lease anywhere, the CLI refuses before writing:
  ```text
  the organism is not running (no body holds the record's lease); start it with `hale dna run`, or ask with --no-wait to leave the ask in the record
  ```
- **Fix:** Start a body (`hale dna dev` or `hale dna run`) in another terminal, or pass `--no-wait` to record the request in Postgres so the organism processes it the next time a body starts.

```text
requested intent …, but the organism journaled no answer in time (record revision …)
```

- **Cause:** `hale dna task create` wrote the request row, but no admission or refusal row arrived before the timeout. The request remains in the record.
- **Fix:** Verify that a body is running (`hale dna body`) and inspect its terminal output for a `no nerves:` warning ([The nerves](#the-nerves))—a body disconnected from NATS JetStream cannot relay or receive requests.

### No record, or a toolchain that moved

```text
no record in memory for … (run `hale habitat graft`)
```

- **Cause:** The directory contains DNA files, but its record schema does not exist in Postgres memory.
- **Fix:** For a new or unattached checkout, run `hale habitat graft . --attach`. For a second clone of an existing project whose record identity is already pinned in `hale.toml`, run `hale habitat migrate` (or `hale dna upgrade` with `HALE_DNA_MEMORY_DSN_OWNER` set) to write the head DSN into `.hale/dna/memory.env`.

```text
this clone holds a record an earlier toolchain kept on git (refs/dna/journal), and records are not carried forward: re-init a fresh record — delete the old refs (…), remove the `[dna] record` line from hale.toml if it has one, and run `hale habitat graft . --attach`
```

- **Cause:** `hale habitat graft` and `hale dna upgrade` reject clones containing legacy Git-backed journal refs (`refs/dna/journal`).
- **Fix:** Delete the legacy Git refs, remove `[dna] record` from `hale.toml` if present, and re-graft the directory:
  ```sh
  git for-each-ref --format='%(refname)' refs/dna/ | xargs -n1 git update-ref -d
  hale habitat graft . --attach
  ```

```text
… holds the head's DSN and is readable by others (mode …): `chmod 600 …`, or remove it and run `hale habitat migrate …`
```

- **Cause:** `.hale/dna/memory.env` contains the Postgres head role DSN, and its file permissions allow group or world access.
- **Fix:** Restrict permissions with `chmod 600 .hale/dna/memory.env` (or delete the file and re-run `hale habitat migrate`).

```text
embedded dna: … (hale …); vendor/dna was materialized from … — run `hale dna upgrade`
```

- **Cause:** Your installed `hale` binary embeds a newer DNA core digest than the one materialized in `vendor/dna/`.
- **Fix:** Run `hale dna upgrade` to synchronize `vendor/dna/` and `hale.lock`.

### The chain does not verify

```text
journal:    … event(s), chain BROKEN at …
```

- **Cause:** Every row in `record_rows` hashes its predecessor (`sha256(prev|kind|entity|body)`). A reader detected a digest or `prev` mismatch, indicating `record_rows` was modified outside the append-only `record_append` stored procedure.
- **Fix:** Compare the affected rows against a database backup ([Memory and the record](./memory.md)).

### A connection that cannot be made

```text
connect: … could not be read (…); nothing was changed
```

- **Cause:** The target NATS URL is unreachable, or the peer record has not published its identity card to its stream.
- **Fix:** Verify network connectivity and run `hale dna nerves migrate` on the target record to publish its card.

---

## Memory (Postgres)

Working memory and the record reside in Postgres ([Memory and the record](./memory.md)).

### The ledger does not answer

```text
memory:     record + ledger (operations and leases live in memory; THE LEDGER IS UNREACHABLE (…): what is read here is the last projection, and nothing is admitted until it answers)
```

- **Cause:** Postgres is unreachable. `hale dna status` falls back to the last cached local projection, and all new task admissions and head writes are refused until Postgres responds.
- **Fix:** Restore Postgres connectivity (or run `hale habitat migrate` to start local habitat containers) and retry the command.

### The body has no memory

```text
hale dna run: no memory: HALE_DNA_MEMORY_DSN_SPINE is not set (`hale dna dev` hands the host the spine's DSN of memory that `hale habitat migrate` migrated, from HALE_DNA_MEMORY_DSN_OWNER or the organism's allocation in the local habitat); nothing is projected or admitted
```

```text
hale dna dev: the organism's memory allocation in the local habitat (memory-…) is not up: `hale habitat migrate …` brings it up and migrates it; or set HALE_DNA_MEMORY_DSN_OWNER to a Postgres of your own
hale dna dev: the organism has no record identity yet: its memory is the local habitat's allocation named for it (`hale habitat graft` pins one)
```

```text
…/.hale/dna/memory.env held a DSN derived from …, not from the organism's memory in the local habitat (…), and is removed: run `hale habitat migrate …`, which brings the organism's memory up and migrates it
```

```text
hale dna run: this record's operations live in the ledger (adopted at …), and no memory is named here (HALE_DNA_MEMORY_DSN_SPINE); a body without its memory admits nothing — set it, or run under `hale dna dev`
```

- **Cause:**
  - Under `hale dna run`: `HALE_DNA_MEMORY_DSN_SPINE` is unset in the environment.
  - Under `hale dna dev`: the organism's `memory-<id>` container in the local habitat is stopped, or `.hale/dna/memory.env` pointed to a stale DSN.
- **Fix:** Run `hale habitat migrate` (requires `docker compose` on `PATH`) to start and migrate the local habitat allocation, or export `HALE_DNA_MEMORY_DSN_SPINE` (printed by `hale dna memory migrate`) when running `hale dna run`.

### The schema is another toolchain's

```text
hale dna run: memory: memory schema … is at version …; this toolchain needs version …; run `hale habitat migrate <organism>` (with HALE_DNA_MEMORY_DSN_OWNER for a Postgres of your own)
```

```text
… memory schema … is at version …, newer than this toolchain's …; it is not migrated back
```

- **Cause:** The Postgres schema version for this record is behind (or ahead of) the installed `hale` binary.
- **Fix:** If the schema is behind, run `hale habitat migrate <organism>` (with `HALE_DNA_MEMORY_DSN_OWNER` set if using an external Postgres server). Schema migrations never downgrade; if the database schema is newer than your binary, upgrade `hale`.

### A context package would be stale

```text
memory's projection is at row …, before the record's knowledge row …; the spine applies it on its tick
```

```text
hale dna run: memory: projection waits for the record to be shared (the sync did not complete)
```

- **Cause:** A worker requested a Work's context package (`hat`), but the Postgres knowledge graph projection was still behind a knowledge row in the record after waiting 20 seconds.
- **Fix:** Ensure an organism body (`hale dna dev` or `hale dna run`) is running with database connectivity so its tick loop projects pending knowledge rows.

### A write the ledger refused

```text
hale dna: `… …` was not written to the ledger: …
```

- **Cause & Fix by Refusal Reason:**
  - `… retired from this organism (person.retired at row …)`: The specified user has been retired via `hale dna retire`.
  - `task … is handed to …, not to …`: Only the assigned person can mark a handed Task `done`.
  - `fenced: the lease … is not held` / `… expired at … (epoch …) and was not renewed`: The body lease fencing token is stale or expired.
  - `the role … does not write as …: a head writes as its owner's members`: On a multi-owner shared record, the head's `HALE_DNA_MEMORY_DSN_HEAD` belongs to a different organization than the caller.
  - `` `…` is a row of the record, not the ledger ``: Internal routing mismatch; `record_rows` kinds cannot be inserted into `ledger_rows`.

### Protected evidence

```text
no receipt key: memory keeps no protected evidence until hale dna memory migrate with HALE_DNA_RECEIPT_KEY
```

```text
memory already seals protected evidence under another receipt key; bodies sealed under it would no longer open, so it is not replaced
```

```text
receipt file: … was redacted; a redacted body is not filed again
… was redacted …; a redacted body is not kept again
```

- **Cause & Fix:**
  - **Missing receipt key:** Filing or reading `customer` or `confidential` evidence requires an encryption key in Postgres memory. Run `hale dna memory migrate` with `HALE_DNA_RECEIPT_KEY` set (at least 16 characters). Once set, the key cannot be replaced with a different one.
  - **Redacted body:** Once a receipt digest is redacted via `hale dna receipt redact`, its plaintext body can never be re-filed.
  - **`receipt.withheld` instead of `receipt.classified`:** Protected evidence was generated when no Postgres memory was configured; only the digest and class were recorded. Re-generate the evidence while connected to memory.

---

## The nerves

The **nerves** are backed by NATS JetStream ([The nerves](./nerves.md)).

### The body hears nothing

```text
hale dna run: no nerves: HALE_DNA_NATS_URL_SPINE and HALE_DNA_NATS_ORG are not set (`hale dna dev` hands the host the spine's URL of nerves whose stream `hale habitat migrate` created, on HALE_DNA_NATS_URL_OWNER or the organism's allocation in the local habitat); the organization hears nothing the record asks
```

```text
hale dna dev: the organism's nerves allocation in the local habitat is not up (`hale habitat migrate …` brings it up and creates the stream)
```

```text
nerves migrate: the server does not run JetStream (start it with -js)
```

- **Cause & Fix:**
  - Under `hale dna dev`: run `hale habitat migrate` to start the `nerves-<id>` container and create the JetStream stream.
  - Under `hale dna run`: export `HALE_DNA_NATS_URL_SPINE` and `HALE_DNA_NATS_ORG` (printed by `hale dna nerves migrate`).
  - External NATS servers must be started with JetStream enabled (`nats-server -js`).

### The organization does not come to read

```text
hale dna run: the organization did not come to read the nerves within 20s; the facts wait in the stream
```

- **Cause:** The `dna/org` child process failed to connect to NATS or lacks permission to consume its durable consumer (`spine`, or `spine_<owner>` on a shared record).
- **Fix:** Inspect `.hale/dna/org.log` and the NATS allocation's `nats.conf`. Queued messages remain safely in the JetStream stream until the connection succeeds.

### A publish the stream did not take

```text
hale dna run: nerves: …; stopping for the unit to start this node again
```

- **Cause:** NATS JetStream failed to acknowledge a published message before the timeout. The body logs `violation.recorded` (`adapter_undeliverable`) and exits with code `75`.
- **Fix:** Under a provisioned systemd service, the unit restarts the body automatically and replays unanswered requests. Under `hale dna dev`, restart the command manually (and run `hale dna nerves migrate` if the stream was deleted).

### The heart's pull is refused

```text
hale dna run: the heart's pull on its durable … was refused … time(s) while the connection held: the durable, or its stream, is gone; it is made again
```

- **Cause:** The application heart's durable consumer or stream was missing while the NATS connection stayed up.
- **Fix:** The body automatically recreates the durable consumer, logs `violation.recorded` (`pulse_stopped`) once per outage, and continues. Inspect the latest 5 violations with `hale dna board`.

---

## The Spine

See [The spine](./spine.md) for budgets, workflows, and leases.

### The budget is spent

```text
… budget exhausted (spent … of … micro-dollars this day in … call(s))
```

- **Cause:** Cumulative model spend in the current budget window reached `allowance_micros`. New tasks (`hale dna task create`), worker spend allowances (`hale dna work allowance`), and Leader Review evaluations (`attempt.allowance_refused` on `review:<id>`) are refused until the window resets.
- **Fix:** Approve pending Reviews manually as the Board (`hale dna review <id> approve`), wait for the next window, or increase `org_budget()` in `dna/org/models.hl` via a reviewed change:
  ```hale,fragment
  fn org_budget() -> dna::BudgetPolicy {
      return dna::BudgetPolicy { window: "day", allowance_micros: 25000000 };
  }
  ```

### An intent noted and never re-offered

```text
offered before a restart and no Task was born; not re-admitted — ask again
```

- **Cause:** `hale dna history` shows `intent.unrecovered` when an intent was offered immediately prior to a restart before a `task.born` row was committed. To prevent duplicate side effects, the spine never re-offers it automatically.
- **Fix:** Re-run `hale dna task create` if the work is still needed.

### An effect whose outcome is unknown

- **Cause:** An attempt claimed by an `uncertain` performer failed to settle or let its lease expire (`attempt.unresolved`). Automated retries are halted because external tools may have already acted.
- **Fix:** Inspect the attempt's history and resolve it manually:
  ```sh
  hale dna effect resolve attempt:<id> --outcome ok|failed
  ```

---

## The Heart and the Body

See [The heart and the body](./heart.md).

### The body lease is held elsewhere

```text
hale dna run: a body for this record is live on … (ticked …, lease expires in …s); a record admits one body — `hale dna body claim --force` takes it over when that body is gone
```

```text
hale dna: body: live on …, ticked … ago; a record admits one body. `hale dna body claim --force` takes the lease from a body that is gone (it stops itself when it next asserts the lease)
```

```text
hale dna run: the body lease is no longer mine: held by …; a record admits one body — stopping
```

```text
hale dna: body: the lease is held by …, not this clone (…); `hale dna body release --force` releases another's
```

```text
hale dna run: memory cannot be reached, so the body lease cannot be taken; a body runs only under a lease it can prove
```

- **Cause:** Each record permits at most one active body process (enforced via a fenced lease in Postgres memory). Starting a second body exits with code `3`.
- **Fix:** Run `hale dna body` to see which machine/clone holds the lease. If the previous body crashed or its host is gone, take over the lease with:
  ```sh
  hale dna body claim --force
  ```
  (Or release it with `hale dna body release --force`.) Any displaced body process automatically halts the next time it checks its lease token.

### A change that did not survive

- **Rolled-back mutation (`mutation.rolled_back`):** `hale dna history` shows `expression.crashed` (with exit code, fleet instance, and node) followed by `mutation.rolled_back`. The newly expressed application exited during the post-deploy observation window, so the host automatically rolled the genome back to the base revision.
- **`…: never expressed by …`:** A fleet instance failed to report `instance.up` at the new revision within the timeout. Check the `hale node <name>` terminal output for Git fetch or build errors.
- **`…: denied: candidate breaks the fleet`:** The candidate compiled individually, but `hale fleet check` failed a cross-service claim (`mutation.deny` lists the failing step).
- **`mutation.refused` with `candidate moved after review: … != …`:** The sandbox commit changed between Review approval and genome application. Re-submit the task to open a fresh Review.

### No fleet

```text
no `[dna] fleet = "<name>"` in hale.toml: the DNA expresses no fleet here
```

- **Cause:** `hale dna fleet`, `hale dna deploy`, and `hale dna rollback` require a fleet seed name in `hale.toml`.
- **Fix:** Add `[dna] fleet = "<name>"` to `hale.toml` ([The heart and the body](./heart.md)).

---

## Senses and Reflexes

See [Senses and reflexes](./senses.md).

```text
hale dna senses up: the local habitat's senses' store runs under docker compose, which is not on PATH
hale dna senses url: the organism's senses' store in the local habitat is not up (`hale habitat migrate …` brings it up)
```

```text
reflexes: HALE_DNA_NATS_URL_REFLEXES, HALE_DNA_NATS_VAULT_REFLEXES and HALE_DNA_NATS_ORG name the nerves this program fires onto
reflexes: no store (HALE_DNA_SENSES_URL); nothing to read
reflexes: the vault holds no credential … (`hale dna upgrade` provisions the organism's secrets)
```

- **Cause & Fix:**
  - Start the local Prometheus-compatible senses store (`senses-<id>`) with `hale habitat migrate` or `hale dna senses up` (requires `docker compose` on `PATH`).
  - When running reflexes standalone, export `HALE_DNA_NATS_URL_REFLEXES`, `HALE_DNA_NATS_VAULT_REFLEXES`, and `HALE_DNA_NATS_ORG` (from `hale dna nerves migrate`), `HALE_DNA_SENSES_URL` (from `hale dna senses up`), and run `hale dna upgrade` if vault credentials are missing.

---

## The Skin (Vault and Secrets)

See [The skin](./skin.md).

### A secret is missing

```text
  MISSING  forge-token  the forge's token — a person supplies it: `hale dna secret set FORGE_TOKEN`
  MISSING  model-OPENAI_API_KEY  the model's key OPENAI_API_KEY — a person supplies it: `hale dna secret set OPENAI_API_KEY`
```

```text
body: no credential for the model (none of … is in the vault where the body runs); `hale dna secret set <NAME> [--body <user@host>]` puts one there
secret: the forge's token is not in the vault (forge-token); `hale dna secret set FORGE_TOKEN` fills it
```

- **Cause:** Required vault slots are empty. Remember that model API keys are read strictly from vault slots (`model-<NAME>`), never from environment variables.
- **Fix:** Populate human-supplied secrets via `stdin` (pass `--body <user@host>` for a remote body):
  ```sh
  hale dna secret set OPENAI_API_KEY
  hale dna secret set FORGE_TOKEN
  ```
  For machine-generated secrets (Postgres roles, NATS accounts, loopback OIDC secrets), run `hale dna upgrade`.

### A secret the verb will not take

```text
secret set: `…` is no slot of the organism's (FORGE_TOKEN, OIDC_CLIENT_SECRET, or a credential dna/org/models.hl names: …); `hale dna secrets` lists them
secret set: the value never goes on the command line (it would sit in every shell history and process list); name the variable alone and give the value on stdin
secret rotate: … was never set (`hale dna secret set …` first)
```

- **Fix:** Pass only a recognized slot name (`hale dna secrets` lists valid names), pipe or type the secret on `stdin` (never as a CLI argument), and run `secret set` before `secret rotate`.

### A drawn secret is gone

```text
nerves migrate: the vault holds no credential for the nerves' … (`hale dna upgrade` provisions the organism's secrets; `hale dna secrets` lists them)
the memory allocation's password is not provisioned (postgres-owner-… in the vault, and its file beside the allocation; `hale dna upgrade` provisions them, `hale dna secrets` lists them)
```

- **Fix:** Run `hale dna upgrade` to regenerate missing infrastructure passwords in the vault and the local habitat allocation directory (`~/.local/share/hale/habitat/local/allocations/<id>/`).

### Violations

```text
violations: … recorded, each absorbed by its owner (the latest last)
```

- **Cause:** A `violation.recorded` row indicates a runtime fault absorbed by a supervisor closure (`adapter_undeliverable`, `pulse_stopped`, or `lease_unsettled`). `hale dna board` displays the 5 most recent violations.

---

## Legs

See [Legs, hands and voice](./legs.md).

### A performer with no effect class

```text
hale dna work: dna/org/work.hl: the … performer declares no effect class; every performer declares one: effect_free, idempotent or uncertain
```

- **Fix:** Add `effect: "effect_free"`, `"idempotent"`, or `"uncertain"` to every performer constructed in `dna/org/work.hl`.

### Nothing to claim

```text
no admitted attempt awaits a … leg with capabilities `…`: … of that kind outstanding, …
```

- **Cause:** `hale dna work next` found no claimable attempt matching your `--as` position, `--kind`, `--capabilities`, `--classes`, and performer effect class. The refusal message breaks down why each outstanding attempt was skipped.

### The lease is gone

```text
stale: the lease on attempt … expired at …
stale: the lease on attempt … is …'s at token …, not …'s at token …
the lease on attempt … was taken as a performer of effect class `…`; an outcome under it names that class, not `…`
```

```text
the lease … held at token … expired at … with no outcome; the attempt is asked again
```

- **Cause:** The attempt lease expired before `submit` or `renew`, was claimed by another worker token, or `submit --effect` did not match the claim's effect class. When a lease expires without an outcome, the spine logs `violation.recorded` (`lease_unsettled`) once and re-queues the attempt (unless the performer was `uncertain`, which transitions to `unresolved`).

### No allowance

```text
the call would cost … micro-dollars, past what is left of the attempt's allowance (… of …): no call was made
the attempt's allowance of … micro-dollars is spent (…); the budget gate admits no further call
```

- **Cause:** The attempt exhausted its per-attempt spend allowance or the organization's daily budget window ([The budget is spent](#the-budget-is-spent)).

---

## Schedules

See [Schedules](./schedules.md).

```text
hale dna: schedule declare: refused: schedule …: …
hale dna: schedule declare: --every … is no interval (a number and ms, s, m, h or d)
hale dna: schedule pause: no schedule `…` is declared in the record
```

- **Common refusal reasons:**
  - `` nobody holds position:…: it convenes nothing until someone does (`hale dna fill … <holder>`) ``: Assign and ratify a holder for the convener position first.
  - `` no definition `…` in the catalog ``: Run `hale dna definitions` to list valid workflow definitions.
  - `` … takes `…`, which the inputs do not name `` / `its inputs are a JSON object`: Provide all required definition inputs as a JSON object in `--args`.
  - `` cron `…`: a cron expression has five fields (minute hour day-of-month month day-of-week), not … ``: Fix the 5-field UTC cron syntax.
  - `… holds neither the Board nor position:…: declaring a schedule is the Board's, or the convener's own holder's`: Run `--as` a user who holds `position:board` or the convener position.

---

## The Head and the Face

See [The head and the face](./head.md).

### The head will not start

```text
hale dna ui: trusted-local is a fixture's mode (HALE_DNA_TRUSTED_LOCAL=1); a project serves its head under OIDC: `git config dna.principal oidc` with dna.oidc.issuer, dna.oidc.client and dna.oidc.member (dna/face/start.sh sets them up with the stub provider, dna/oidc)
hale dna ui: dna.principal is oidc, so dna.oidc.issuer, dna.oidc.client and dna.oidc.redirect must be set
hale dna ui: the issuer must be https (plain http only on this machine): …
hale dna ui: the issuer … is not usable: …
```

- **Fix:** Start the local head via `hale dna head <project>` (which configures the local stub OIDC provider automatically), or configure an HTTPS OIDC issuer (or a loopback issuer with a pinned `dna.oidc.key`):
  ```sh
  git config dna.principal oidc
  git config dna.oidc.issuer https://issuer.example
  git config dna.oidc.client acme-head
  git config dna.oidc.redirect https://head.example/callback
  git config --add dna.oidc.member "<subject>=alice"
  hale dna secret set OIDC_CLIENT_SECRET
  ```

### A verdict refused

```text
review … refused the verdict: …
```

- **Refusal causes & fixes:**
  - `digest mismatch: … != …`: The candidate digest changed or `--digest` was stale. Re-run `hale dna review <id>` and approve the current digest.
  - `authority … does not satisfy …`: Pass an `--authority` that meets or exceeds the Review's required authority (`board` for Board-level Reviews).
  - `reviewer … authored the candidate`: Code changes require two-person review (someone other than the candidate's author must approve).
  - `reviewer … holds no position this Review requires (…)`: The Review is routed (`hale dna route`); only holders of the required signer positions may approve it.
- **`hale dna: review: a verdict cites no gate run; the host observes each gate's run at the forge (`hale dna github sync`)`:** Remove `--evidence`; CI gate runs are observed automatically from the forge.

### Every Review needs the Board

- **Cause:** By default, tasks without an explicit class are planned as `application` changes, while the default `dna::Grant` in `dna/org/main.hl` only delegates `"refactor docs"` to the Leader.
- **Fix:** Approve Reviews as the Board (`hale dna review <id> approve`), or widen `grant.classes` in `dna/org/main.hl` via a reviewed change ([Shaping and governing it](./shaping.md)).

### A person's task that never settles

- **Cause:** A human-assigned Task only settles when its assignee runs `hale dna task done <id> --as <assignee>` with any required evidence (`--evidence <digest>`) or third-party exception (`--exception <why> --authorized-by <who>`). Inspect open human tasks with `hale dna board`.

---

## Starting Over with a Fresh Record

Because the record lives in a dedicated Postgres schema (`dna_<record-identity>`) while applied code changes remain in `git log`, you can reset a project to a clean record without losing its source code:

1. Remove the `record = "…"` line under `[dna]` in `hale.toml`.
2. (If the clone still has legacy `refs/dna/` Git refs, delete them—see [No record, or a toolchain that moved](#no-record-or-a-toolchain-that-moved).)
3. Re-graft the existing directory to provision a fresh record identity and Postgres schema:
   ```sh
   hale habitat graft . --attach
   ```

See [Reference: Where things live](./reference.md#where-things-live) for a complete map of all state locations.
