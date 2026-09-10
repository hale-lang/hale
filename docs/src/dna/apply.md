# Apply, restart, observe

Approval applies **exactly the reviewed candidate**, and nothing
else moves the genome.

## Apply

When a mutation's Review settles `approve`, the assembly:

1. reads the request the Journal recorded — the candidate commit the
   reviewer looked at and the worktree it lives in;
2. checks that the worktree's head is still that commit. A candidate
   that moved after the review is **refused by digest**
   (`mutation.refused: candidate moved after review`), and nothing
   is applied;
3. takes the Mutation's lease (a fencing token; a stale one is
   refused);
4. applies through the gateway: a fast-forward to the candidate, or
   a merge of the pinned commit when the genome moved on. The
   request is journaled under the candidate's own idempotency key
   *before* dispatch and the result appended once, so a crashed and
   retried apply reads its own record and never commits twice
   (`applied (replayed)`, and no second restart). A merge that does
   not complete is aborted; there is no half-applied state;
5. journals `mutation.applied` and asks the host for a restart,
   naming the candidate and the fitness signals the proposal
   declared.

Rejection or revision journals `mutation.rejected` / `mutation.revise`
and touches nothing. The Mutation, its candidate commit, its
Attempts and its evidence stay in the Journal; the worktree, which
was scratch, is dissolved.

```text
   41  review.settled         m1   approve by riley
   42  effect.requested       apply:dbbb49f5…   m1
   43  effect.result          apply:dbbb49f5…   ok
   44  mutation.applied       m1   dbbb49f550f3f021f390710803e48f1b3d1593ff
   45  expression.restart_requested m1   apply dbbb49f5… fitness guests_greeted +
```

## Restart

The applied genome is not yet the running expression. `hale dna run`
sees the restart request in the Journal and:

- cuts a fresh artifact (`current.topology`, keeping the old one as
  `previous.topology`) and rebuilds. If the candidate does not
  build, the host reports `build_failed` on the membrane and the
  running organism rolls back (below);
- terminates the old organism (SIGTERM, then SIGKILL after five
  seconds) and starts the new one with `HALE_DNA_RESTART_FOR=<id>`
  and `HALE_DNA_EXPRESSION=<shape> build <digest>`. The new
  expression journals `expression.restarted` itself, at birth: the
  fact that a restart happened is the expression's to record, not
  the host's;
- relaunches iris with the diff from the previous artifact to the
  current one, so the review view shows what just changed and the
  fleet view rings any instance still expressing the old artifact.

```text
hale dna run: m1 requests a restart (apply dbbb49f5… fitness guests_greeted +)
hale dna run: organism restarted (pid 1694392) as 8517c3db7499d3b3 build b1ac50c8c3ef
```

## The observation window

`--observe <secs>` (default 15) is how long the new expression must
stay up. At the end the host publishes `ExpressionObserved` on the
membrane — `healthy` with the shape hash it observed, or what went
wrong — and the organism decides, against the Mutation's state:

- **`healthy`**: `mutation.retained`, the worktree dissolved, and
  the originating pressure re-measured (`pressure.remeasured`,
  naming the Task and the fitness signals);
- **anything else**: the genome is rolled back to the Mutation's
  base (`git reset --keep`, through the gateway, journaled as
  `mutation.rolled_back`) and the host is asked for the old
  expression back — a restart request marked `rollback`, which the
  host answers without opening another window.

Only an applied Mutation is judged; a late or repeated report
changes nothing.

```text
hale dna run: m1 observed healthy for 5s as 8517c3db7499d3b3
   47  expression.observed    m1   healthy 8517c3db7499d3b3 up for 5s
   48  pressure.remeasured    m1   task t1 fitness guests_greeted +: healthy up for 5s
   49  mutation.retained      m1   observed healthy as 8517c3db7499d3b3
```

## When the expression crashes

If the new expression exits inside the window there is nobody left
to decide, so the host accounts for it explicitly rather than
silently: it appends `expression.crashed` and `mutation.rolled_back`
to the Journal itself (the Journal has one writer at a time, and
the organism is gone), resets the genome to the base, rebuilds, and
restarts the base expression. The Mutation ends `rolled_back` with
the exit code in its history.

## What this phase does not do

Live state migration. A restart is a restart: the old process
ends, the new one starts from its constructors. An application that
must carry state across a change carries it the way it would
across any deploy. Mutations that alter live state layout are a
different problem (`reperspective` covers the footprint-preserving
case; the rest is future work), and DNA does not pretend otherwise.
