# Troubleshooting

**`organism: not running — reading the Journal`** in `status`. No
host is up here. `status`, `review <id>`, `history`, `board`,
`fleet` and `ui` work from the record; `task create` and a verdict go into
the record when the repository has a remote (an organization
elsewhere answers), and need `hale dna run` or `dev` in another
terminal when it doesn't.

**`the organism is not running here (no membrane) and the
repository has no remote to reach one through`**. Same thing, with
nowhere to send it. Start a host, or add a remote and start one
somewhere.

**`membrane client failed: … File name too long`**. The sockets
under `.hale/dna/` are unix sockets, and a path over about a hundred
bytes cannot be bound. Move the checkout somewhere shorter.

**`task t1 born … [failed]`**, and `history t1` says
`credential not present`. The editor's hosted model has no key. Set
the key the catalog names (`ANTHROPIC_API_KEY` or `OPENAI_API_KEY`;
`hale dna models` shows which backends are permitted), or point the
editor's router at a local or scripted model in `dna/org/models.hl`
— [Shaping it](./shaping.md).

**`needs leader`, and nothing happens.** The Leader decides with a
model; with no key it cannot, and the review waits. Answer it as the
Board (`hale dna review m1 approve --as you`), or give the Leader a
model. The same happens when the budget is spent: `history` shows
`budget.exhausted`, `task create` is refused with the spend, and the review
is yours until the next window.

**`task t1 born … [planning]`**, and it stays there. The Leader is
asked what kind of work the intent is before it is admitted, and the
Leader decides with a model. With no key, or a spent budget, the
word never comes; give the Leader a model or answer as the Board.
The intent is not lost and is not offered twice.

**`intent.unrecovered` in `history`.** An intent was offered before
a stop and no admission names it. It is noted once and never
re-offered, because work may already have run for it. Ask again if
you still want it.

**A handed Task that never settles.** A person's job is a case, and
only that person's `hale dna task done <id> --as <them>` closes it —
with the evidence or the authorized exception its acceptance
practice requires (`hale dna board` says which). A restart does not
close it; neither does anyone else's report.

**`refused: budget exhausted (spent … of … micro-dollars this
day …)`** from `task create`. The organization's allowance for the window is
spent; nothing model-backed is routed until the next one. Raise
`allowance_micros` in `org_budget()` (`dna/org/models.hl`), or wait.

**`needs board`** on every review. Requests through `task create` are
`application` changes, and the default grant is `refactor docs`.
It's the expected posture for a new organization: you are asked, and
told why. Widen `classes` in `dna/org/main.hl` when you want the
Leader's judgement on them.

**`disposition stage`.** Within the grant, but the evidence doesn't
reach the bar — usually a first change in a fresh history, or a
program with no tests. Still a normal review; the bar rises and
falls with the evidence.

**`review m1 refused the verdict: digest mismatch`.** The verdict
named a candidate other than the one under review — the sandbox
changed, or a `--digest` was stale. Re-render with `hale dna review
m1` and answer again; the digest in the `decide:` line is the one to
name.

**`authority reviewer does not satisfy board`.** The verdict's
`--authority` doesn't meet what the review requires. Board-class
changes, and anything touching law, effects or the fleet's shape,
need the Board.

**`reviewer editor authored the candidate`.** The name on the
verdict is the author's. Use your own.

**`mutation.refused: candidate moved after review`** in the
history. The sandbox's commit changed between the review and the
apply. Nothing was applied. Ask again; the new candidate gets its
own review.

**`m1 [rolled_back]`.** Either an expression exited inside the
observation window (`history m1` shows `expression.crashed` with the
exit code — and, on a fleet, the instance and node), or you rejected
a post-review change. The repository is back at the commit the
change started from, and the old expression is running.

**`never expressed by api-1`.** A touched instance never reported
`instance.up` at the revision within the settle time. Look at that
node's terminal: the revision may not have fetched, or the seed may
not build there.

**`no [dna] fleet = "<name>" in hale.toml`.** `hale dna fleet`,
`deploy` and `rollback` need to know which declared fleet the DNA
expresses. Add the section — [Operating the fleet](./operating.md).

**`candidate breaks the fleet`.** The candidate's services compose,
but a claim the plan makes over them fails. The witness is in the
`fleet` receipt of the denied mutation.

**`journal: … chain BROKEN at <head>`.** At the head this status
loaded (`chain_head` in `--json`), `refs/dna/journal` has more or fewer
commits than rows. A record that grew since the load is not broken;
this one's history was rewritten. `git reflog
refs/dna/journal` finds the old head; `hale dna sync` from a clone
that has it restores the rest.

**`THE LEDGER IS UNREACHABLE (…)`** on `status`'s `memory:` line.
Memory — the Postgres behind the ledger — is not answering. What you
read here — `status`, the board, `history` — is the last projection
this body built, and **nothing is admitted until memory answers**.
The organism does not stop the instant memory goes: the fence keeps
the lease it last proved and stops the body at the margin before
that lease expires, so a short outage costs nothing. Requests made
from a clone are not lost either: each is a `ledger.requested` row in
the record, and the spine admits it once memory is back.

**`no memory: HALE_DNA_MEMORY_DSN_SPINE is not set`** from the host.
It runs with no memory: nothing is projected into the graph, no
request is admitted, and a context package is empty and says so.
Under `hale dna dev`, bring memory up (`docker compose` on `PATH`,
or `HALE_DNA_MEMORY_DSN_OWNER`); under `hale dna run`, set the spine's
DSN, which `hale dna memory migrate` prints. On a record that has
adopted the ledger, `hale dna run` refuses to start at all without
it — a body without its memory admits nothing.

**`memory schema dna_… is at version N; this toolchain needs version
M`** (or `has no schema version`). The host refuses to start on
memory another toolchain migrated, or none did. Run `hale dna memory
migrate` with the owner's DSN in `HALE_DNA_MEMORY_DSN_OWNER`. A
migration refuses a schema a newer toolchain wrote; upgrade the
toolchain instead.

**`memory's projection is at row N of the record's M; the spine
applies it on its tick`**. A context package waited 20 seconds for
the graph to catch up with the record and refused rather than hand
over a stale one. Is a body running with memory, and does one of them
hold the spine lease? `.hale/dna/spine.json` on each body says
whether it holds it and what it has projected; `spine.taken` and
`spine.lost` in `hale dna history spine` say who held it when.

**`ledger.request_refused` in the history.** The spine refused a
request a head made. Its body says why — a retired person, a task
handed to someone else, a transfer accepted outside the owner it
was offered to, a claim already taken (`claimed`), a decision read at
a ledger revision that has since moved (`stale_revision`), a record
kind asked of the ledger, or over a shared record a request not
signed with its owner's key. Read it, decide again, and run the verb
again: a refusal is final for that request.

**A request that stays requested.** The spine admits requests on its
tick, so a `ledger.requested` row with neither an admitted row nor a
refusal after it means no spine is running on this record, or it
cannot reach memory. `sync`, then look at the body (`hale dna body`,
`.hale/dna/spine.json` there).

**`ledger.adopting` in the history with no `ledger.adopted`.** The
adoption was asked for and not yet carried out — no body holding the
spine lease has run since, or memory went away mid-copy. Nothing is
broken and nothing is lost: the spine picks it up on its next tick,
and because every row is keyed by its commit nothing is copied twice.
Or run `hale dna ledger abandon --why <why>`, which empties what was
copied and leaves the organism on the record alone. Either way the
record keeps every operational row it ever held.

**`no receipt key: memory keeps no protected evidence until …`**. A
customer or confidential body was filed, or read, with no receipt key
in memory. Run `hale dna memory migrate` with `HALE_DNA_RECEIPT_KEY`
(sixteen characters at least) in the owner's environment. A
different key from the one memory already holds is refused: bodies
sealed under the first would no longer open.

**`… was redacted; a redacted body is not kept again`** (or `… not
filed again`). A body the record redacted is refused when anyone
files it again, in git or in memory — memory keeps the digest of
every body it erased.

**`receipt.withheld`** where you expected `receipt.classified`. A
protected body was produced where no memory was named, so nothing
could keep it. The record holds its digest and class, and no body
exists anywhere; produce the evidence again where memory is.

**`the record diverged, and local event <commit> (… by <author>) was
signed with <key>, not this clone's`** from `sync`. Under `dna.trust
= signed`, a reconcile that would rebuild a row this clone did not
sign refuses as a whole, before any ref moves: rebuilding it would
re-sign someone else's row as yours. Nothing was discarded — every
local row and commit stays where it is, and the remote is untouched.
Two ways out, both in the message: that row's writer syncs first (a
writer rebuilds its own rows), or you fast-forward once the remote
holds it.

**Two organizations on one record.** One `hale dna run` per
repository. Two hosts syncing one remote will both relay and both
answer.

**Where things are.** `refs/dna/journal` (the record),
`refs/dna/receipts/<sha256>` (receipts), `refs/dna/lease/*`,
`refs/dna/revisions/<rev>` (what nodes fetch); `.hale/dna/` (sockets,
sandboxes, scratch, `status.json`, `spine.json`); `.hale/node/<name>/`
on a node (pid files, artifacts). Once the organism has adopted the
ledger, the day's work is not under any of these: it is in memory,
the record's own schema in Postgres, and `hale dna ledger` says where
things stand.

**Starting over.** Delete the refs and the record is gone; `git log`
keeps every change it applied:

```sh
git for-each-ref --format='%(refname)' refs/dna | xargs -n1 git update-ref -d
hale dna init .        # seeds a fresh record from the program as it is now
```
