# Troubleshooting

**`organism: not running — reading the Journal`** in `status`. No
host is up here. `status`, `review <id>`, `history`, `board`,
`fleet` and `ui` work from the record; `ask` and a verdict go into
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
`OPENAI_API_KEY`, or point the router at a local or scripted model —
[Shaping it](./shaping.md).

**`needs leader`, and nothing happens.** The Leader decides with a
model; with no key it cannot, and the review waits. Answer it as the
Board (`hale dna review m1 approve --as you`), or give the Leader a
model.

**`needs board`** on every review. Requests through `ask` are
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

**`journal: … chain BROKEN`.** `refs/dna/journal` has more or fewer
commits than rows. Someone rewrote the branch. `git reflog
refs/dna/journal` finds the old head; `hale dna sync` from a clone
that has it restores the rest.

**Two organizations on one record.** One `hale dna run` per
repository. Two hosts syncing one remote will both relay and both
answer.

**Where things are.** `refs/dna/journal` (the record),
`refs/dna/receipts/<sha256>` (receipts), `refs/dna/lease/*`,
`refs/dna/revisions/<rev>` (what nodes fetch); `.hale/dna/` (sockets,
sandboxes, scratch, `status.json`); `.hale/node/<name>/` on a node
(pid files, artifacts).

**Starting over.** Delete the refs and the record is gone; `git log`
keeps every change it applied:

```sh
git for-each-ref --format='%(refname)' refs/dna | xargs -n1 git update-ref -d
hale dna init .        # seeds a fresh record from the program as it is now
```
