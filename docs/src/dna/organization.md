# The organization

What oversees your code is an org chart, and the org chart is a
Hale program: `dna/org/main.hl`. Positions are loci, routing is the
bus, a position's capabilities are its effect contract, and the
compiler holds the law in `dna/org/law.hl` against the wiring you
actually built. You edit it like any source, and it changes itself
the way the code does — through a review.

## Who is in it

```text
Board ─── the human authority
  │        intent enters through it; escalations and reports leave through it;
  │        it owns every grant, and its verdicts carry `--authority board`
  ├── Leader ─── the model-backed position holding the codebase's grant
  │              decides the Reviews inside it, leaves the rest to the Board;
  │              every decision is a model call with evidence in the record
  └── substrate ─── the record, the gateways, verification, the editing
                    position, the Reviews; it acts, it never decides
```

**The Board is you** — and whoever else answers with the Board's
authority: from a terminal (`--as`), from the page, from a GitHub
login named in `dna.github.board`. Its queue:

```text
$ hale dna board
board: 15 review(s) need your verdict
  purpose  ratify the declared purpose?
  k:79c636063701  ratify the design practice `design/principles`: Minimal structure: add a…
  … (the eight practices of the design, one Review each)
  k:638e61e51c84  ratify the operating practice `operating/row-first`: every live signal…
  … (the six operating practices, one Review each)
leader: 1 review(s) inside the grant, being decided
decide with `hale dna review <id> approve|revise|reject`; `hale dna review <id>` renders one
```

The toolchain seeds two families of practice, and the Board decides
each practice on its own. The design says how an organization is
shaped. The operating practices say how the organism runs: what the
Leader plans within, and what a reviewer cites.

- `operating/one-store-per-step`: a step writes one store, by its one
  writer; no distributed transaction anywhere.
- `operating/row-first`: every live signal is a record row before it
  is sent, consumed once by its id.
- `operating/readings-never-act`: a reading from the senses never
  acts; what matters becomes a pressure or concern row.
- `operating/legs-hold-nothing`: a leg holds nothing between tasks.
- `operating/deploy-settles-on-pulse`: a deploy settles on the heart's
  own first event, or rolls back as a new step.
- `operating/the-forge-decides`: what merges is decided at the forge,
  by people, and comes back as one verdict row.

**The Leader** holds a grant the Board wrote into `main.hl`:

```hale,fragment
boundary: dna::AutonomyBoundary {
    child: "chat",
    grant: dna::Grant { child: "chat", classes: "refactor docs", max_magnitude: 4, review: "pre" }
},
review_policy: dna::OrgPolicy { },
```

One person may hold more than one role. Propose a practice as the
person doing the work (`hale dna practice propose … --as you`) and
ratify it as the Board (`hale dna review … approve --as you
--authority board`): the record keeps who proposed and who decided,
and nothing refuses the two being the same person. A team that wants
an independent approver for practices would need a review policy
saying so; none ships.

The Leader is the organism's **architect**: it proposes and the
Board decides, and it thinks at two moments. When an ask enters, it
reads its brief — the charter, the purpose, the law as the genome
holds it, and the practices the Board has ratified — and says what
the ask is: which kind of thing it concerns (the organism, an
appendage, a product, or a person's job), what class of change, on
what target, one change or several. The admission of the ask binds
that word (`workflow.admitted`, its inputs), and the Mutation carries
the class it named. A plan only splits and classifies; it never widens
what you asked.

On a cadence you set, the Leader walks the machinery rather than the
work: it reads the record's signals — asks it could not plan, concerns
piling up, grants that contracted, verdicts refused, rollbacks — and
either proposes one small change to the organization, which enters as
an ordinary ask for you to decide, or records that the state is clean.
It never proposes a large restructure unprompted, and never creates
work for the sake of activity.

The live organization's loop calls
`self.core.request_tick(std::time::monotonic_ns() / 1000000)`.
This queues cadence processing with incoming work, so a task cannot enter
while a journal refresh is rebuilding its cached view. After upgrading an
older project, replace `self.core.tick(` with `self.core.request_tick(` in
`dna/org/main.hl`'s loop; keep the same millisecond clock. `hale dna upgrade`
reports this change but leaves your organization's source for you to edit.
Synchronous `tick` remains available for isolated callers or code already
running in the owner's handler. Organizations in the same process use
distinct `org_id` values for cadence and plan routing.

Some asks are nobody's software change. When the Leader plans one as a
person's job, the organization hands it on and the record keeps it
*handed* — to the person the Leader named — until that person reports
it done: `hale dna task done <id> --as <you> --note "what happened"`.
One row, in their name, and only theirs: someone else is refused and
told to `hale dna task reassign <id> --to <them>` first. When a person
leaves, `hale dna retire <who> --to <successor>` moves everything they
hold, as rows, and refuses to drop any of it.

A practice can come from anyone, and the Board decides it. When the
same thing keeps going wrong, write down how it should go and say what
prompted it:

```text
hale dna practice propose acceptance/expense-receipt \
    --text "An expense closes with its receipt.
evidence: required" --because "three bills arrived without receipts"
```

The organization records who proposed it and why, and opens a Review
for the Board; `hale dna practice` lists the practices with their state.

When the Board has ratified an acceptance practice for a kind of job
that says `evidence: required`, a job of that kind closes only with its
evidence: `hale dna task done t41 --as mara --evidence sha256:…`, a
receipt filed with `hale dna receipt file`. When the evidence cannot
exist, someone else authorizes the exception: `--exception "the driver
gave no receipt" --authorized-by dana`. The practice is fixed when the
job is handed, so changing it later leaves the jobs already waiting
under the contract they were handed with. A completion with neither is
shown as human-reported: it says the job is done, not that anything was
verified.

`dna/acceptance/books` is a worked example of all of this: a capture-only
intake that files bills as evidence and asks for the receipts that did
not come with them. Its acceptance test (`dna/tests/books_slice_test.hl`)
runs the first books slice against a live organism: bills arrive, the
payer is handed each missing receipt, the organism restarts while they
wait, a completion needs its evidence or an authorized exception, the
repeated omissions lead to a reviewed change to the intake and a new
version of the practice, the next bill uses both while an older case
keeps its contract, and a week is exported from the record alone.

Often the decision on such a job is not the person's to make, and is
not made in Hale: a client approves an expense by mail, a manager
agrees on a call. File what shows it, then report the decision as
theirs:

```text
hale dna receipt file approval.eml --as mara
hale dna task decide t41 --decided-by dana@client --via mail --evidence sha256:… --as mara
```

The record says "reported by mara, decided by dana@client, via mail",
never that mara approved it, and it refuses a report that names the
reporter as the decider. Whether a report settles the job is the
Board's call, made once for a kind of obligation: ratify a practice
named `acceptance/expense-approval` whose text says `reported
decisions: allowed`, and reports settle expense approvals. Without such
a practice the report is kept, the Task waits for the assignee's own
`task done`, and `hale dna board` says why.

Some work leaves the organization altogether: the year-end ledger goes
to your accountant, who runs their own organism on their own record.
Sync never does this. It copies your whole record to people inside it.
Instead, connect the two records, and the connection says what may
cross:

```text
hale dna connect git@example.com:acct/record.git --name acct --as accountant \
    --purpose "year-end books" --classes internal,customer
hale dna review c-acct-41 approve --authority board    # someone else on the Board
hale dna handoff acct task t7
```

The Task arrives in the accountant's mailbox for your record — never
their journal, and nothing of their record comes back to you but their
acceptances — with where it came from, its history here and its purpose. A receipt crosses as its digest, never its
body, and only if the connection carries its class. Their side admits it
only through a connection of their own back to you, and when they run
`hale dna handoff accept`, your `hale dna handoff sync` settles the Task.
`hale dna disconnect acct --why "the engagement ended"` stops anything
further crossing, and both records keep what already did.

A grant sits under the organization's own, when one is written: a
child's grant is read through the organism's *current* grant at every
assessment, so when the organization's leash shortens on repeated
failure, every child's shortens with it, and the record says whom it
now binds. A child written wider than the organization's grant is
refused at birth and bound by it.

A change of a class in the grant and under its size is the Leader's
to decide. It reads the same brief above the source diff and the
semantic diff with the deep model tier, answers `approve`, `revise`
or `reject` with its reasoning, and the record has both: the call (which model, what it
cost, the digests) and the reasoning in full (`review.reasoned`,
rendered as `why:` by `hale dna review <id>`). The Board can answer first, or overrule nothing — a
settled Review is settled — but it widens or narrows the grant in a
reviewed commit, and the record shows who did.

## Whose positions they are

One organization, one owner: every position is its own, and the file
`dna/org/owners` the scaffold writes beside `main.hl` stays empty. When
a record is shared between firms (stage B2), the map names an owner per
position and each owner's members:

```
org = acme
org/collections = north
acme: alice, carol
north: bob
```

A position not named takes its nearest named ancestor's owner. Each
body says which owner it is (`git config dna.owner acme`) or `hale dna
run` refuses to start it, and it admits intents only for positions its
owner holds. `hale dna task create --to org/collections` from acme's clone does
not go to acme's body: it goes into the record, where north's controller
relays it, and `hale dna status` in acme's clone shows it as
`[unadmitted]` with north's name until then. Changing the map is a
mutation of the organization that every affected owner approves, each
through one of its members; one rejection settles it. Each owner's body
holds its own lease in memory, `owner/<owner>` (a shared record runs
no body until its ledger is adopted), and a write the body makes after
its lease was taken over is refused `fenced`, whatever the body still
believes.
Each owner mints its own ids (`acme:t3`, `acme:m2`) and claims its own
effects (`acme:apply:<candidate>`), so nothing two owners do contends
for one name; a birth the store says is already claimed is minted again.
A job planned for another owner's member is offered, not handed: the
Task waits as `transfer_requested` until one of that owner's members
runs `hale dna task accept <id>`, and their controller hands it on
from there. Only the receiving owner settles a transfer. Money is each
owner's own: a grant names who pays (`funder: "acme/ops"`), every
allocation is reserved once by the store, every attempt's spend is
retained, and a purchase two owners fund is two reservations that may
not both land — undoing one is a compensation someone authorizes.
Every owner runs a body, and every body is a node of the spine: each
projects the record, carries out what the record asks, and erases
redacted evidence, none coordinating
([Operating](./operating.md#the-spine-is-every-node)). Each owner's
heads write the ledger as that owner's role in memory, which writes
only in its own members' names; the migration makes a role per owner
from the owners' keys (`HALE_DNA_OWNER_KEYS`, `<owner>=<key> …`), and
the members are the genome's owners map, which every node projects
([Operating](./operating.md#shared-records-and-owners-roles)). The map may
also name a `host = <owner>`, changed only with every owner's
approval; it no longer decides where anything runs.

## Who decides what

| change class | examples | decided by |
|---|---|---|
| `docs`, `refactor` (in the grant) | a comment, a rename, a split | the Leader |
| `application` | what an ask produces by default | the Board, until the grant says otherwise |
| `organization` | a new position, a changed route in `dna/org` | the Board |
| `constitutional` | `dna/org/law.hl`, the application's constitution | the Board |
| `process-policy` | workflow definitions, the review policy, the grant itself | the Board |
| `topology` | the fleet plan, `hale.toml` | the Board |

And whatever the class, a candidate that touches law, reaches a new
effect class, or crosses ownership goes to the Board. That is the
policy `OrgPolicy` encodes; it is a locus in `main.hl` you can
replace.

## The law

`dna/org/law.hl` is what no position may ever do, as the compiler
sees it:

```hale,fragment
constitution Org {
    apply_only_through_the_substrate: forbid reaches(positions, effects(genome_apply)) avoiding substrate;
    editors_never_commit: forbid reaches(editors, effects(repo_write));
    editors_never_touch_worktrees: forbid reaches(editors, effects(worktree_io));
    editors_never_apply: forbid reaches(editors, effects(genome_apply));
    editors_never_learn: forbid reaches(editors, knowledge);
    leader_never_commits: forbid reaches(leader, effects(repo_write)) avoiding substrate;
    leader_never_touches_worktrees: forbid reaches(leader, effects(worktree_io)) avoiding substrate;
    credentials_sealed: require sealed(all credentials);
}
```

Hand the editor a git handle in `main.hl` and `hale check` fails
with the path that proves it. Add clauses of your own in the same
block; don't weaken these.

## How it grows

Pressure is a typed signal — something a metrics relay, an operator,
or you saw more than once:

```sh
hale dna pressure raise ingest "the ingest worker lags behind the queue every evening"
hale dna pressure                         # what was raised, and what was answered
```

When one source has raised it three times (`appendage_threshold`),
the organization journals `appendage.proposed` and tells the Board:
an organ for it is proposed, not grown. With `initiative: true` in
`main.hl`, it goes one step further on its own: an `organization`
mutation, in a sandbox, adding the position to `dna/org` — a
candidate commit, verified like any other, that lands in the Board's
queue. Approve it and the organization restarts itself with the new
position in it; the host does the restart and watches the window
exactly as it does for the application.

Nothing is added to the org chart without the Board. The org chart
grows the way the code does, and the record has both.

## Reports

```text
$ hale dna report
report r37 filed: since #0: proposed 1 · reviewed 2 · applied 1 · retained 1 · rolled back 0 · rejected 0 · escalated 0 · pressure 0 · proposals 0 · model calls 2 (20 µ$) · settled: purpose approve by riley, m1 approve by riley
```

A report is a row in the record for the Board: what happened since
the last one, what it cost, what was escalated. File one whenever
you want the summary; the numbers come from the record, not from
anyone's account of it.
