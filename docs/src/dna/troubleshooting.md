# Troubleshooting

**`organism: not running — reading the Journal`** in `status`. The
host isn't up. `hale dna status`, `review <id>` and `history` work
offline from the memory; `ask` and a verdict need `hale dna run` in
another terminal.

**`the organism is not running (no membrane)`** from `ask` or a
verdict. Same thing. If the host *is* running, look at its terminal:
the program may have exited, or not have bound its sockets within
twenty seconds (`the membrane did not come up`). A program whose
`run()` returns takes the organism with it.

**`task t1 born … [failed]`**, and `history t1` says
`credential not present`. The editor's hosted model has no key. Set
`OPENAI_API_KEY`, or point the router at a local or scripted model —
[Shaping it](./shaping.md).

**`disposition escalate`** on every review. Requests through `ask`
are `application` changes, and the default grant is
`refactor docs`. It's the expected posture for a new organism: you
are asked, and told why. Widen `classes` in `dna/assembly.hl` when
you want the organism's own assessment on the record.

**`disposition stage`.** Within the grant, but the evidence doesn't
reach the bar — usually a first change in a fresh history, or a
program with no tests. Still a normal review; the bar rises and
falls with the evidence.

**`review m1 refused the verdict: digest mismatch`.** The verdict
named a candidate other than the one under review — the sandbox
changed, or a `--digest` was stale. Re-render with `hale dna review
m1` and answer again; the digest in the `decide:` line is the one
to name.

**`authority agent does not satisfy maintainer`.** The verdict's
`--authority` (default `maintainer`) doesn't meet what the review
requires. Reviews of changes that touch rules or effects need a
maintainer; a post-review refactor accepts a `reviewer`.

**`reviewer editor authored the candidate`.** The name on the
verdict is the author's. Use your own.

**`mutation.refused: candidate moved after review`** in the
history. The sandbox's commit changed between the review and the
apply. Nothing was applied. Ask again; the new candidate gets its
own review.

**`m1 [rolled_back]`.** Either the program exited inside the
observation window (`history m1` shows `expression.crashed` with
the exit code), or you rejected a post-review change. The
repository is back at the commit the change started from, and the
old program is running. The candidate commit is still in the
history if you want to look at it.

**`hale check --matrix` fails after `init`.** Read the message: a
`forbid reaches` over your program with an unresolvable edge fails
closed, and `init` will have written `organism_gated` commented out
with the reason in `dna_constitution.hl`. Fix the edge (usually an
indirect call the model cannot follow) and uncomment it.

**`journal: … chain BROKEN`.** `.hale/dna/journal.jsonl` was
edited or truncated. It is the organism's memory, not a log to tidy;
restore it from wherever it went, or delete `.hale/dna` and start the
memory over (`hale dna init .` re-seeds it; your code is untouched).

**Two organisms on one repository.** Don't. They would share sockets
and a memory. One `hale dna run` per checkout.

**Where things are.** `.hale/dna/journal.jsonl` (the memory),
`.hale/dna/evidence/` (receipts by hash, plus each change's diffs),
`.hale/dna/worktrees/<id>/` (sandboxes of changes still in flight),
`.hale/dna/status.json` (what iris reads), `.hale/dna/*.sock` (the
door).

**Starting over.** `rm -rf .hale/dna` forgets everything the
organism knew; `git log` keeps every change it applied. Then
`hale dna init .` seeds a fresh memory from the program as it is now.
