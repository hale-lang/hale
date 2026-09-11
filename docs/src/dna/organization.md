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
board: 1 review(s) need your verdict
  purpose  ratify the declared purpose?
leader: 1 review(s) inside the grant, being decided
decide with `hale dna review <id> approve|revise|reject`; `hale dna review <id>` renders one
```

**The Leader** holds a grant the Board wrote into `main.hl`:

```hale,fragment
boundary: dna::AutonomyBoundary {
    child: "chat",
    grant: dna::Grant { child: "chat", classes: "refactor docs", max_magnitude: 4, review: "pre" }
},
review_policy: dna::OrgPolicy { },
```

A change of a class in the grant and under its size is the Leader's
to decide. It reads the source diff and the semantic diff with the
deep model tier, answers `approve`, `revise` or `reject` with its
reasoning, and the record has both: the call (which model, what it
cost, the digests) and the reasoning in full (`review.reasoned`,
rendered as `why:` by `hale dna review <id>`). The Board can answer first, or overrule nothing — a
settled Review is settled — but it widens or narrows the grant in a
reviewed commit, and the record shows who did.

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
