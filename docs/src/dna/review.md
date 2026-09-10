# The Review in detail

A Review is a locus, not a flag. It owns the exact question, the
candidate's digest at decision time, the authority required, and the
verdicts it received with their provenance. It settles only on a
verdict that names the exact candidate, comes from a reviewer whose
authority satisfies the requirement, and is independent of the
authoring Attempt. A capable position with the wrong authority does
not settle it, and neither does the transport.

```sh
hale dna review              # the pending Reviews, one line each
hale dna review m1           # source diff · semantic diff · evidence table · magnitude
hale dna review m1 --iris    # the same diff in iris, beside the status and the membrane form
hale dna review m1 approve --as riley --comment "fine"
hale dna review m1 approve --digest bf94e503c1c0   # name the candidate you looked at
hale dna review m1 approve --no-wait               # send it; the answer lands in the record
```

The render works **offline** — it reads the record and the
receipts — so a reviewer can look from any clone, before the
organization is even running, and a Review requested by an
organization that has since been restarted is still there to answer
(pending mutation Reviews are re-born from the record at birth).

## Who may answer

| authority | rank | held by |
|---|---|---|
| `board` (also `maintainer`) | 4 | a person: `--as` from the terminal, the page, a GitHub login in `dna.github.board` |
| `leader` | 3 | the Leader position, deciding with a model inside the grant |
| `supervisor` | 2 | a position the org chart grows, over one part of the codebase |
| `reviewer` | 1 | a GitHub review from a login not on the Board; a post-review refactor's answer |

A verdict satisfies a requirement when its rank is at least the
required rank, so the Board can always answer, and the Leader can
answer what a supervisor or reviewer could. `OrgPolicy` decides what
each Review requires: `leader` inside the grant; `board` for
Board-class changes and for anything that touches law, widens
effects or crosses ownership.

## What a reviewer sees

```text
$ hale dna review m1
review m1 [pending]: apply m1 (docs): document the chat server in main.hl?
  needs leader · candidate bf94e503c1c002f277248b14b6afd1910bb8ce6f
  mutation m1 (docs) by editor · disposition under the grant: stage · shape 3c9b9327e480d349
  magnitude: novelty 3

source diff (git 232dc8f18dde .. bf94e503c1c0):
   main.hl | 1 +
   1 file changed, 1 insertion(+)

  diff --git a/main.hl b/main.hl
  --- a/main.hl
  +++ b/main.hl
  @@ -28,3 +28,4 @@ main locus Chat {
   fn main() {
       Chat { };
   }
  +// documented by the organism: Echo answers every Ping

semantic diff (hale model diff, baseline .. candidate):
  classification: source-only  (shape_hash 3c9b9327e480d349 -> 3c9b9327e480d349)
  legend: + added  - removed  ~ renamed  > moved  * split/joined  ? ambiguous  ! changed in place
  no semantic differences

evidence (fmt=0 check=0 verify=0 test=0 diff=0 rollback=0 fleet=0):
  step       ok     code  receipt        bytes
  base       yes    0 b676facc8c41   26
  fmt        yes    0 e3b0c44298fc   0
  check      yes    0 e3b0c44298fc   0
  verify     yes    0 e3b0c44298fc   0
  test       yes    0 184a3407ce31   67
  fleet      yes    0 b33b2e832b21   277
  rollback   yes    0 89978376be81   96
  diff       yes    0 8b0b8da30d3a   3002
  receipts: refs/dna/receipts/<digest> (git cat-file -p)

decide: hale dna review m1 approve|revise|reject|abstain [--as <you>] [--comment <c>] [--digest bf94e503c1c0]
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
  receipt per row. `fleet` is every plan the workspace declares,
  composed with the candidate's artifacts. A row that says `NO` is
  why the disposition is what it is.

The Leader reads the same three views. Its verdict is a model call
in the record (`model.called` with the deep tier, the digests of what
it read, the cost) and a `review.verdict` with `authority: leader`.

The kill test that shaped this — two rounds of reviewers deciding
from the semantic diff alone, the source diff alone, and both — is
recorded in `dna/kill-test/WALKTHROUGH.md`. Its short form: the
source diff and the combined view decided every case; the semantic
diff alone completed the law-backed cases and correctly held on the
rest. So the combined view is the product, for people and for the
Leader alike, and the semantic view is never offered as a standalone
approval interface.

The magnitude line is the boundary's vector, never a score:
affected loci, contract change, effects widened, law touched,
placement or ownership change, state migration, external blast
radius, reversibility, novelty against the accepted lineage.

## The verdict

```text
$ hale dna review m1 approve --as riley --comment "fine"
review m1 settled: approve by riley
```

The verdict is a typed `ReviewVerdict` keyed to this Review — over
the membrane when the organization is here, as a `review.verdict`
row in the record otherwise. What the Review checks, in order:

1. **The digest.** The verdict names the candidate the reviewer
   looked at — by default the one the request pinned, or the one
   given with `--digest`. Any other is refused:
   `review.refused: digest mismatch`. This is the pin against a
   candidate that changed after the reviewer looked; the apply path
   checks it again against the worktree's actual head.
2. **The authority.** A verdict whose rank does not reach the
   requirement is refused by name:
   `authority reviewer does not satisfy board`.
3. **Independence.** The author of the candidate cannot review it.

Every answer is a row: `review.settled` with the outcome and the
reviewer, or `review.refused` with the reason. `revise` and `reject`
settle the Review too, and leave the genome and the expression
untouched. `abstain` is recorded and leaves the Review open.

An `approve` on a mutation Review is also the apply. The next
chapter is what that does.

## On GitHub

With `git config dna.github owner/repo`, the host (and `hale dna
github sync`) mirrors every pending mutation Review to a pull
request: the candidate pushed to `dna/<id>`, the three views above
as the body, `github.pr` in the record. Every GitHub review on it
becomes a `review.verdict` row in the reviewer's login — `board` when
`dna.github.board` lists the login, `reviewer` otherwise — once, keyed
by login, commit and state, and the Review admits or refuses it
exactly as above. The settlement goes back as a comment
(`github.commented`), and an approval pushes the genome so GitHub
sees the merge. GitHub is a projection of the record and the record
wins.

## On the page and in iris

`hale dna ui` renders this same text under *Review* and sends the
verdict form the way the CLI does. Iris, attached by the host, shows
the pending Review in its organism panel with the candidate the
verdict must name, and `hale dna review m1 --iris` opens the review
perspective on the Mutation's semantic diff document. Either way the
Review decides; the surfaces only publish.
