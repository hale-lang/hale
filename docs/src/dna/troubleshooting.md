# Troubleshooting

Each entry starts with what the organism prints, word for word, with
`…` where a name, a number or a digest goes. Then the cause, then what
to do. The entries are grouped by the part that owns the failure.

A verb that fails prints `hale dna: ` and the reason. A running body
prints its lines as `hale dna run: ` or `hale dna dev: `, and a node as
`hale node <name>: `.

Four verbs answer with nothing running, and most questions start with
one of them:

```sh
hale dna status            # the projection: organism, journal, reviews, body, memory
hale dna board             # what waits on the Board, secrets missing, the latest violations
hale dna history t1        # every row about one entity, by its causal links
hale dna secrets           # every secret the organism needs, and whether the vault holds it
```

## The record

### Nothing is running here

```text
organism:   not running — reading the Journal
```

`status` says this when no body in this clone holds the record's
lease. It is not an error: `status`, `history`, `board`, `review <id>`
and `ui` read the record. Asking for work needs an organism to answer.
With a remote, the ask goes into the record and a body elsewhere
answers it. Without one, start a body here with `hale dna run` (or
`hale dna dev`) in another terminal.

```text
the organism is not running here (no body in this clone holds the record's lease) and the repository has no remote to reach one through; start it with `hale dna run`
```

Same cause, and nowhere to send the ask. Start a body, or add a remote
with a body behind it.

```text
requested intent …, but the organism journaled no answer in time (record revision …)
```

`hale dna task create` wrote the ask and no admission or refusal came
back in time. The ask stays in the record. Check that a body is
running (`hale dna body`), and look at its terminal for a `no nerves:`
line ([The nerves](#the-nerves)): a body that hears no nerves relays
nothing.

### No record, or a toolchain that moved

```text
no record at refs/dna/journal in … (run `hale dna init`)
```

The directory has DNA files but no record. Run `hale dna init` in it.

```text
embedded dna: … (hale …); vendor/dna was materialized from … — run `hale dna upgrade`
```

The first line of `status`. The toolchain you are running embeds a
different DNA source from the one `vendor/dna` was copied from. Run
`hale dna upgrade`, which copies this toolchain's core into
`vendor/dna` again.

### The chain does not verify

```text
journal:    … event(s), chain BROKEN at …
```

`refs/dna/journal` is one commit per row. At the head `status` loaded,
the number of commits differs from the number of rows: the ref's
history was rewritten. `git reflog refs/dna/journal` lists the heads
the ref had before; a clone that still holds the old head has the
rows. See [Memory and the record](./memory.md).

### A fetch loses a ref lock

```text
fetch origin: … cannot lock ref …
```

From `sync`, `ledger adopt` or a body. Another git process held a ref
the fetch was updating. The record tries such a fetch five times
before it gives up, and nothing changed. Run the command again. If it
keeps failing, look for a stuck `git` in that clone, or a stale
`.lock` file under `.git/refs/dna/` left by one that was killed.

### A sync refuses to re-sign someone else's row

```text
the record diverged, and local event … (… …, by …) was signed with …, not this clone's; a reconcile would re-sign it as this clone's, so the record was not changed and every local row is kept at refs/dna/journal. Have that row's writer sync first — a writer rebuilds its own rows — then sync again to fast-forward; or fetch and fast-forward once the remote holds it
```

Under `dna.trust = signed`, `hale dna sync` would have to rebuild a row
this clone did not sign. It refuses before any ref moves; nothing is
lost. Do what the message says.

### A connection that cannot be made

```text
connect: … could not be read (…); nothing was changed
connect: no record identity at … (`refs/dna/identity`; a record publishes it when it syncs — `hale dna sync` there)
```

The first: the URL is wrong or unreachable from here. The second: the
other record exists but has never synced, so it has not published its
identity. Run `hale dna sync` in the other record, then connect again.

## Memory

Working memory is Postgres. [Memory and the record](./memory.md)
covers the ledger, the roles and protected evidence.

### The ledger does not answer

```text
memory:     record + ledger (routing 1, adopted at …; operations and leases live in memory; THE LEDGER IS UNREACHABLE (…): what is read here is the last projection, and nothing is admitted until it answers)
```

The `memory:` line of `status`, on an organism that adopted the
ledger. What you read is the last projection this clone built, and no
work is admitted until memory answers. A head's write made meanwhile
is refused, not queued: run the verb again once memory is back.

### The body has no memory

```text
hale dna run: no memory: HALE_DNA_MEMORY_DSN_SPINE is not set (`hale dna dev` applies memory's schema from HALE_DNA_MEMORY_DSN_OWNER or dna/compose.yaml and hands the host the spine's DSN); nothing is projected or admitted
```

Under `hale dna run` the body is given the spine role's DSN, which
`hale dna memory migrate` prints. Set it. Under `hale dna dev`, memory
comes from `dna/compose.yaml`, and a line before this one says why it
did not come up:

```text
hale dna dev: dna/compose.yaml is here but `docker compose` is not on PATH; set HALE_DNA_MEMORY_DSN_OWNER to a Postgres of your own
hale dna dev: no dna/compose.yaml and no HALE_DNA_MEMORY_DSN_OWNER: there is no memory to run on (`hale dna upgrade` writes the compose file)
```

Once the ledger is adopted, a body without memory does not start:

```text
hale dna run: this record's operations live in the ledger (adopted at …), and no memory is named here (HALE_DNA_MEMORY_DSN_SPINE); a body without its memory admits nothing — set it, or run under `hale dna dev`
```

### The schema is another toolchain's

```text
hale dna run: memory: memory schema … is at version …; this toolchain needs version …; run `hale dna memory migrate` with the owner's DSN in HALE_DNA_MEMORY_DSN_OWNER
```

Also `… has no schema version …` when nothing migrated it. Run
`hale dna memory migrate` with the owner's DSN. A migration never goes
backwards:

```text
… memory schema … is at version …, newer than this toolchain's …; it is not migrated back
```

Upgrade the toolchain instead.

### A context package would be stale

```text
memory's projection is at row …, before the record's knowledge row …; the spine applies it on its tick
```

A piece of work asked memory for its context, and memory had not yet
applied a knowledge row the record holds. It waited 20 seconds, then refused
rather than hand over a stale package. Check that a body is running
with memory and that its sync completes. A body that cannot share the
record says so:

```text
hale dna run: memory: projection waits for the record to be shared (the sync did not complete)
```

### A write the ledger refused

```text
hale dna: `… …` was not written to the ledger: …
```

Memory's insert function refused the row, and the rest of the line
says why. The reasons include:

- `… retired from this organism (person.retired at row …)`: the person
  named has left.
- `task … is handed to …, not to …`: only the assignee closes their
  task.
- `fenced: the lease … is not held`, or `… expired at … (epoch …) and
  was not renewed`: the write named a body lease that is no longer
  live.
- `the role … does not write as …: a head writes as its owner's
  members`: over a shared record, this head's DSN is another owner's.
  Check which `HALE_DNA_MEMORY_DSN_HEAD` it was given.
- `` `…` is a row of the record, not the ledger ``: a record kind was
  asked of the ledger.

### An adoption that has not happened yet

`history` shows `ledger.adopting` and no `ledger.adopted`. The
adoption was asked for and no node has carried it out: none has run
since, or memory went away mid-copy. A node picks it up on its next
tick. To leave the organism on the record alone instead,
`hale dna ledger abandon --why <why>` asks a node to empty the ledger.

### Protected evidence

```text
no receipt key: memory keeps no protected evidence until hale dna memory migrate with HALE_DNA_RECEIPT_KEY
```

A customer or confidential body was filed or read, and memory holds no
key to seal it under. Run `hale dna memory migrate` with
`HALE_DNA_RECEIPT_KEY` (sixteen characters or more) in the owner's
environment. A second, different key is refused, because what the
first sealed would no longer open:

```text
memory already seals protected evidence under another receipt key; bodies sealed under it would no longer open, so it is not replaced
```

A body the record redacted cannot come back, in git or in memory:

```text
receipt file: … was redacted; a redacted body is not filed again
… was redacted …; a redacted body is not kept again
```

A `receipt.withheld` row where you expected `receipt.classified`
means a protected body was produced where no memory was named. The
record keeps its digest and class; no body exists anywhere. Produce the
evidence again where memory is.

## The nerves

The nerves are NATS JetStream. See [The nerves](./nerves.md).

### The body hears nothing

```text
hale dna run: no nerves: HALE_DNA_NATS_URL_SPINE and HALE_DNA_NATS_ORG are not set (`hale dna dev` creates the stream from HALE_DNA_NATS_URL_OWNER or dna/compose.yaml and hands the host the spine's); the organization hears nothing the record asks
```

The body runs, and every ask stays unanswered. Under `hale dna run`,
set the two variables `hale dna nerves migrate` prints. Under
`hale dna dev`, a line before this one says why the nerves did not
come up:

```text
hale dna dev: dna/compose.yaml is here but `docker compose` is not on PATH; set HALE_DNA_NATS_URL_OWNER to a NATS server of your own
hale dna dev: no dna/compose.yaml and no HALE_DNA_NATS_URL_OWNER: there are no nerves to run on (`hale dna upgrade` writes the compose file)
```

A server of your own must run JetStream:

```text
nerves migrate: the server does not run JetStream (start it with -js)
```

### The organization does not come to read

```text
hale dna run: the organization did not come to read the nerves within 20s; the facts wait in the stream
```

The organization's connection cannot reach the server, or its user
may not read its durable consumer (`spine`, or `spine_<owner>` over a
shared record; `dna/nats.conf` lists the users). Its log,
`.hale/dna/org.log`, says which. Nothing is lost: the facts wait in the
stream until it connects.

### A publish the stream did not take

```text
hale dna run: nerves: …; stopping for the unit to start this node again
```

The stream did not acknowledge a publish in time. The body records a
`violation.recorded` row (`adapter_undeliverable`), stops and exits
75. A provisioned body's systemd unit starts it again, and every
request still unanswered is relayed again. Under `hale dna dev` there
is no unit: start it again yourself. If the stream itself is gone,
`hale dna nerves migrate` makes it.

### The heart's pull is refused

```text
hale dna run: the heart's pull on its durable … was refused … time(s) while the connection held: the durable, or its stream, is gone; it is made again
```

Recorded once per outage as `violation.recorded` (`pulse_stopped`).
The body makes the durable again and goes on. `hale dna board` lists
the latest five violations.

## The spine

See [The spine](./spine.md) for the budget, the workflow engine and
the body lease.

### The budget is spent

```text
… budget exhausted (spent … of … micro-dollars this day in … call(s))
```

`task create` is refused with this, `history` shows
`budget.exhausted`, and nothing model-backed runs until the window
turns. The Leader's decision on a Review is refused by the same gate
(`attempt.allowance_refused` on `review:<id>`), so the Review waits
for the Board; answer it yourself with `hale dna review <id> approve`.
A leg's allowance is refused the same way. The allowance is
`org_budget()` in `dna/org/models.hl`:

```hale,fragment
fn org_budget() -> dna::BudgetPolicy {
    return dna::BudgetPolicy { window: "day", allowance_micros: 25000000 };
}
```

Raise it in a reviewed change, or wait for the next window.

### An intent noted and never re-offered

`history` shows `intent.unrecovered`:

```text
offered before a restart and no Task was born; not re-admitted — ask again
```

Work may already have run for it, so it is never offered twice. Ask
again with `hale dna task create` if you still want it.

### An effect whose outcome is unknown

An attempt claimed by an `uncertain` performer that never reported is
`unresolved`: its effect may or may not have happened, and no program
retries it. A person decides:

```sh
hale dna effect resolve attempt:<id> --outcome ok|failed
```

## The heart and the body

See [The heart and the body](./heart.md).

### The body lease is held elsewhere

A record admits one body. A body that starts while another holds the
lease stops at once, with exit code 3:

```text
hale dna run: a body for this record is live on … (ticked …, lease expires in …s); a record admits one body — `hale dna body claim --force` takes it over when that body is gone
```

`hale dna body` says who holds it. If that body is gone (its machine
died, its process was killed), take the lease:

```sh
hale dna body claim --force
```

Without `--force`, a claim on a live lease is refused:

```text
hale dna: body: live on …, ticked … ago; a record admits one body. `hale dna body claim --force` takes the lease from a body that is gone (it stops itself when it next asserts the lease)
```

The body that lost its lease stops when it next asserts it:

```text
hale dna run: the body lease is no longer mine: held by …; a record admits one body — stopping
```

Releasing another clone's lease also needs `--force`:

```text
hale dna: body: the lease is held by …, not this clone (…); `hale dna body release --force` releases another's
```

A body that cannot reach the record's remote does not start, because
it cannot prove a lease:

```text
hale dna run: the record's remote cannot be reached, so the body lease cannot be taken; a body runs only under a lease it can prove
```

### A change that did not survive

A Mutation that `rolled_back`: `history` shows `expression.crashed`
with the exit code (on a fleet, the instance and node), then
`mutation.rolled_back`. The expression exited inside its observation
window; the genome is back at the base and the old expression runs.

```text
…: never expressed by …
```

A touched instance never reported `instance.up` at the new revision in
time. Look at that node's terminal: the revision may not have fetched,
or the seed may not build there.

```text
…: denied: candidate breaks the fleet
```

The candidate's services compose, but a claim the fleet plan makes over
them fails. The `mutation.deny` row names the failing steps.

`mutation.refused` with `candidate moved after review: … != …`: the
sandbox's commit changed between the review and the apply. Nothing was
applied; ask again, and the new candidate gets its own Review.

### No fleet

```text
no `[dna] fleet = "<name>"` in hale.toml: the DNA expresses no fleet here
```

`hale dna fleet`, `deploy` and `rollback` need to know which declared
fleet to express. Add the section to `hale.toml`; see
[The heart and the body](./heart.md).

## Senses and reflexes

See [Senses and reflexes](./senses.md).

```text
hale dna senses up: dna/compose.yaml is here but `docker compose` is not on PATH
hale dna senses up: no dna/compose.yaml: there is no store for the senses (`hale dna upgrade` writes the compose file)
```

The senses' store is the `senses` service of `dna/compose.yaml`.
Under `hale dna dev` the same reason comes as `hale dna dev: senses: …`,
and nothing else stops for it.

```text
reflexes: HALE_DNA_NATS_URL_REFLEXES, HALE_DNA_NATS_VAULT_REFLEXES and HALE_DNA_NATS_ORG name the nerves this program fires onto
reflexes: no store (HALE_DNA_SENSES_URL); nothing to read
reflexes: the vault holds no credential … (`hale dna upgrade` provisions the organism's secrets)
```

The reflexes refuse to start without their nerves, their store, or
their credential. `hale dna nerves migrate` prints the first three
variables, and `hale dna senses up` the store's URL.

## The skin

See [The skin](./skin.md) for the vault and its slots.

### A secret is missing

```text
  MISSING  forge-token  the forge's token — a person supplies it: `hale dna secret set FORGE_TOKEN`
  MISSING  model-OPENAI_API_KEY  the model's key OPENAI_API_KEY — a person supplies it: `hale dna secret set OPENAI_API_KEY`
```

`hale dna secrets` lists every secret the organism needs and marks the
ones the vault lacks. A secret the organism draws itself (memory's
roles, the nerves' accounts, a loopback OIDC client) says
`` — `hale dna upgrade` draws it ``. A slot is yours to fill, from
stdin:

```sh
hale dna secret set OPENAI_API_KEY
```

Add `--body <user@host>` to fill the vault where the body runs. The
board repeats what is missing:

```text
body: no credential for the model (none of … is in the vault where the body runs); `hale dna secret set <NAME> [--body <user@host>]` puts one there
secret: the forge's token is not in the vault (forge-token); `hale dna secret set FORGE_TOKEN` fills it
```

A model key is read from its vault slot `model-<NAME>` only, never
from the environment. With none, a call is refused before it is made,
and the `model.called` row in `history` says `credential not present`.

### A secret the verb will not take

```text
secret set: `…` is no slot of the organism's (FORGE_TOKEN, OIDC_CLIENT_SECRET, or a credential dna/org/models.hl names: …); `hale dna secrets` lists them
secret set: the value never goes on the command line (it would sit in every shell history and process list); name the variable alone and give the value on stdin
secret rotate: … was never set (`hale dna secret set …` first)
```

Name the slot alone, give the value on stdin, and set a secret before
you rotate it.

### A drawn secret is gone

```text
nerves migrate: the vault holds no credential for the nerves' … (`hale dna upgrade` provisions the organism's secrets; `hale dna secrets` lists them)
the compose database's password is not provisioned (postgres-owner-… in the vault, dna/postgres.secrets beside compose; `hale dna upgrade` provisions them, `hale dna secrets` lists them)
```

Run `hale dna upgrade`, which draws what the vault lacks. A password
file (`dna/postgres.secrets`, `dna/nats.secrets.conf`) is never
written where git would track it:

```text
refused: … is not ignored by git, and a password is never written where git would track it (add /… to .gitignore; `hale dna upgrade` does)
refused: … is tracked by git, and a password is never written into a tracked file (`git rm --cached …`)
```

### Violations

```text
violations: … recorded, each absorbed by its owner (the latest last)
```

A `violation.recorded` row is a closure a part absorbed and went on
from: `adapter_undeliverable` and `pulse_stopped`
([The nerves](#the-nerves)), `lease_unsettled` ([Legs](#legs)).
`hale dna board` shows the latest five.

## Legs

`hale dna work` prints one JSON object per verb; a refusal has
`"state": "refused"` and the reason. See [Legs, hands and
voice](./legs.md).

### A performer with no effect class

```text
hale dna work: dna/org/work.hl: the … performer declares no effect class; every performer declares one: effect_free, idempotent or uncertain
```

Every performer in `dna/org/work.hl` declares `effect_free`,
`idempotent` or `uncertain`. Until it does, the verbs that claim or
hand back an outcome refuse to start; the others still read, settle,
file friction and drain.

### Nothing to claim

```text
no admitted attempt awaits a … leg with capabilities `…`: … of that kind outstanding, …
```

`hale dna work next` found nothing it may take, and says why for each
attempt it passed over: performed in the organism, awaiting the owner,
needing a capability the leg lacks, `not admitting a performer of
effect class …`, of a data class the leg may not see, or `unresolved`
([An effect whose outcome is unknown](#an-effect-whose-outcome-is-unknown)).

### The lease is gone

```text
stale: the lease on attempt … expired at …
stale: the lease on attempt … is …'s at token …, not …'s at token …
the lease on attempt … was taken as a performer of effect class `…`; an outcome under it names that class, not `…`
```

An outcome, a renewal or an allowance under a lease that is no longer
this leg's. Renew before the lease ends (`hale dna work renew`). An
outcome names the effect class the lease was taken with.

When a lease expires with no outcome, the owner records a
`violation.recorded` row (`lease_unsettled`) once for that lease, and
asks the attempt again:

```text
the lease … held at token … expired at … with no outcome; the attempt is asked again
```

### No allowance

The budget's one gate answers `hale dna work allowance`. On a spent
window it refuses with `budget exhausted (…)`
([The budget is spent](#the-budget-is-spent)). Within an attempt, a
call that would pass what is left is not made:

```text
the call would cost … micro-dollars, past what is left of the attempt's allowance (… of …): no call was made
the attempt's allowance of … micro-dollars is spent (…); the budget gate admits no further call
```

## Schedules

See [Schedules](./schedules.md).

```text
hale dna: schedule declare: refused: schedule …: …
```

The organization refused the declaration and said why. The reasons
are:

- `` nobody holds position:…: it convenes nothing until someone does (`hale dna fill … <holder>`) ``:
  fill the convening position first.
- `` no definition `…` in the catalog ``: `hale dna definitions` lists
  the catalog.
- `` … takes `…`, which the inputs do not name ``: pass every input the
  definition takes in `--args`.
- `its inputs are a JSON object`: `--args` is a JSON object.
- `` cron `…`: a cron expression has five fields (minute hour day-of-month month day-of-week), not … ``.
- `… holds neither the Board nor position:…: declaring a schedule is
  the Board's, or the convener's own holder's`.

```text
hale dna: schedule declare: --every … is no interval (a number and ms, s, m, h or d)
hale dna: schedule pause: no schedule `…` is declared in the record
```

`hale dna schedule` lists what the record declares, with each
schedule's occurrences, skips and misses.

## The head and the face

See [The head and the face](./head.md).

### The head will not start

```text
hale dna ui: trusted-local is a fixture's mode (HALE_DNA_TRUSTED_LOCAL=1); a project serves its head under OIDC: `git config dna.principal oidc` with dna.oidc.issuer, dna.oidc.client and dna.oidc.member (dna/face/start.sh sets them up with the stub provider, dna/oidc)
```

A head needs a principal source and refuses to start without one.
Name the OIDC issuer in the record's git config, and put the client's
secret in the vault:

```sh
git config dna.principal oidc
git config dna.oidc.issuer https://issuer.example
git config dna.oidc.client acme-head
git config dna.oidc.redirect https://head.example/callback
git config --add dna.oidc.member "<subject>=alice"
hale dna secret set OIDC_CLIENT_SECRET
```

The secret goes to the vault as `oidc-client-<client>`. The head checks
its configuration before it serves:

```text
hale dna ui: dna.principal is oidc, so dna.oidc.issuer, dna.oidc.client and dna.oidc.redirect must be set
hale dna ui: the issuer must be https (plain http only on this machine): …
hale dna ui: the issuer … is not usable: …
```

An issuer on the loopback is not usable until the record pins its key
(`git config dna.oidc.key <SubjectPublicKeyInfo, base64>`).

### A verdict refused

```text
review … refused the verdict: …
```

The Review did not admit the verdict, and says why:

- `digest mismatch: … != …`: the verdict named a candidate other than
  the one under review (the sandbox changed, or a `--digest` was
  stale). Render it again with `hale dna review <id>` and answer the
  digest it shows.
- `authority … does not satisfy …`: the Review needs a higher
  authority (`board` for a Board-class change).
- `reviewer … authored the candidate`: sign in your own name.
- `reviewer … holds no position this Review requires (…)`: a routed
  Review admits only holders of the positions `hale dna route` names.

```text
hale dna: review: a verdict cites no gate run; the host observes each gate's run at the forge (`hale dna github sync`)
```

A verdict does not carry gate evidence. The host reads each gate's run
at the forge.

### Every Review needs the Board

`status` shows `[pending] needs board — …` on each Review. That is the
default posture: an ask is an `application` change, and the Leader's
grant in `dna/org/main.hl` covers `refactor docs`. Decide with
`hale dna review <id> approve`, or widen the grant in a reviewed
change; see [Shaping and governing it](./shaping.md).

### A person's task that never settles

A handed Task closes only when its assignee reports it with
`hale dna task done <id> --as <them>`, carrying the evidence or the
authorized exception its acceptance practice requires. A restart does
not close it, and neither does anyone else's report. `hale dna board`
lists the tasks waiting.

## Starting over

Deleting the record's refs deletes the record; `git log` still holds
every change it applied. `hale dna init` then seeds a fresh record from
the program as it is now, and keeps the files already there:

```sh
git for-each-ref --format='%(refname)' refs/dna | xargs -n1 git update-ref -d
hale dna init .
```

Where each piece of state lives is in the
[Reference](./reference.md#where-things-live).
