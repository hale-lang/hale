# Reviewing

A Review is a locus, not a flag. It owns the exact question, the
candidate's digest at decision time, the authority required, and
the verdicts it received with their provenance. It settles only on
a verdict that names the exact candidate, comes from a reviewer
whose claimed authority satisfies the requirement, and is
independent of the authoring Attempt. A capable performer with the
wrong authority does not settle it, and neither does the transport.

```sh
hale dna review              # the pending Reviews, one line each
hale dna review m1           # source diff · semantic diff · evidence table · magnitude
hale dna review m1 --iris    # the same diff in iris [4], beside the status and the membrane form
hale dna review m1 approve --as riley --comment "fine"
hale dna review m1 approve --digest dbbb49f550f3   # name the candidate you looked at
```

The render works **offline** — it reads the Journal and the
receipts — so a reviewer can look before the organism is even
running, and a Review requested by an organism that has since been
restarted is still there to answer (pending mutation Reviews are
re-born from the Journal at birth).

## What a reviewer sees

```text
$ hale dna review m1
review m1 [pending]: apply m1 (application): document the chat server in main.hl?
  needs maintainer · candidate dbbb49f550f3f021f390710803e48f1b3d1593ff
  mutation m1 (application) by editor · disposition under the grant: escalate · shape 8517c3db7499d3b3
  magnitude: novelty 3

source diff (git 4319af28e30e .. dbbb49f550f3):
   main.hl | 1 +
   1 file changed, 1 insertion(+)

  diff --git a/main.hl b/main.hl
  --- a/main.hl
  +++ b/main.hl
  @@ -110,3 +110,4 @@ main locus ChatServer {
       }
   }
   fn main() { ChatServer { }; }
  +// documented by the organism: the rooms are the only way to the signer

semantic diff (hale model diff, baseline .. candidate):
  classification: source-only  (shape_hash 8517c3db7499d3b3 -> 8517c3db7499d3b3)
  legend: + added  - removed  ~ renamed  > moved  * split/joined  ? ambiguous  ! changed in place
  no semantic differences

evidence (fmt=0 check=0 verify=0 test=0 diff=0 rollback=0):
  step       ok     code  receipt        bytes
  base       yes    0     e607a8e25cfc   26
  fmt        yes    0     e3b0c44298fc   0
  check      yes    0     e3b0c44298fc   0
  verify     yes    0     e3b0c44298fc   0
  test       yes    0     184a3407ce31   67
  rollback   yes    0     a06effb044d9   96
  diff       yes    0     0dfce6838a75   70257
  receipts under .hale/dna/evidence

decide: hale dna review m1 approve|revise|reject|abstain [--as <you>] [--comment <c>] [--digest dbbb49f550f3]
```

Three views, always together, because each sees something the
others cannot:

- **The source diff** is git's, base to candidate. It is the only
  view that sees handler behaviour — a `+1` becoming `+100`, a
  changed literal, an added but unread field.
- **The semantic diff** is `hale model diff --text` from its
  receipt: declarations added, removed, renamed, moved, split;
  contracts that moved (a param, a publication, a topic's payload
  shape); effect classes newly reached; claims whose result flipped.
  When a law goes from `holds` to `violated`, it is one line here
  and a page of reasoning from the source.
- **The evidence table** is what the toolchain established, with a
  receipt per row. A row that says `NO` is why the disposition is
  what it is.

The kill test that shaped this — two rounds of reviewers deciding
from the semantic diff alone, the source diff alone, and both — is
recorded in `dna/kill-test/WALKTHROUGH.md`. Its short form: the
source diff and the combined view decided every case; the semantic
diff alone completed the law-backed cases and correctly held on the
rest. So the combined view is the product, and the semantic view is
never offered as a standalone approval interface.

The magnitude line is the boundary's vector, never a score:
affected loci, contract change, effects widened, law touched,
placement or ownership change, state migration, external blast
radius, reversibility, novelty against the accepted lineage. Here
the change moved nothing in the model, so only `novelty 3` shows —
nothing had been accepted into this lineage yet.

## The verdict

```text
$ hale dna review m1 approve --as riley --comment "the rooms stay the only way"
review m1 settled: approve by riley
```

The verdict is a typed `ReviewVerdict` published on the membrane
and keyed to this Review. What the Review checks, in this order:

1. **The digest.** The verdict names the candidate the reviewer
   looked at — by default the one the request pinned, or the one
   given with `--digest`. Any other is refused:
   `review.refused: digest mismatch`. This is the pin against a
   candidate that changed after the reviewer looked; the apply path
   checks it again against the worktree's actual head.
2. **The authority.** `--authority` defaults to `maintainer`. A
   verdict whose authority does not satisfy the requirement is
   refused by name: `authority agent does not satisfy maintainer`.
3. **Independence.** The author of the candidate cannot review it.

Every answer is a Journal event: `review.settled` with the outcome
and the reviewer, or `review.refused` with the reason. `revise` and
`reject` settle the Review too, and leave the genome and the
expression untouched. `abstain` is recorded and leaves the Review
open.

An `approve` on a mutation Review is also the apply. The next
chapter is what that does.

## In iris

`hale dna run` attaches iris with the organism's status projection,
so the same pending Review is in the organism panel (`5`) with its
evidence line and the candidate the verdict must name, and the
membrane form (`m`) sends the same typed verdict the CLI does. For
the diff itself, `hale dna review m1 --iris` opens the review
perspective (`4`) on the Mutation's semantic diff document beside
the status and the form. Either way the organism decides; iris only
publishes.
