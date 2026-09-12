# Apply, express, observe

Approval applies **exactly the reviewed candidate**, and nothing
else moves the genome.

## Apply

When a mutation's Review settles `approve`, the substrate:

1. reads the request the record holds — the candidate commit the
   reviewer looked at and the worktree it lives in;
2. checks that the worktree's head is still that commit. A candidate
   that moved after the review is **refused by digest**
   (`mutation.refused: candidate moved after review`), and nothing is
   applied;
3. checks that the genome is still the base the review was against. If
   a maintainer committed, or another proposal landed, while this one
   was under review, it is **refused** (`mutation.refused: the genome
   moved since the review`) and nothing is applied. Propose again on
   the new base and the whole gate runs on that result. An apply the
   record already holds is exempt: a retried apply replays (step 5),
   and the head it left behind is the candidate, not the base;
4. takes the Mutation's lease (a fencing token; a stale one is
   refused);
5. applies through the gateway: a **fast-forward to the candidate, or
   nothing**. The request is journaled under the candidate's own
   idempotency key *before* dispatch and the result appended once, so
   a crashed and retried apply reads its own record and never commits
   twice (`applied (replayed)`, and no second restart);
6. journals `mutation.applied` and expresses it — through the
   deployment gateway when one is wired, or by asking the host,
   naming the candidate, the seed the change edited and the fitness
   signals the proposal declared.

Rejection or revision journals `mutation.rejected` / `mutation.revise`
and touches nothing. The Mutation, its candidate commit, its
Attempts and its evidence stay in the record; the worktree, which was
scratch, is dissolved.

```text
   28  review.settled         m1   approve by riley
   29  effect.requested       apply:bf94e503…   m1
   30  effect.result          apply:bf94e503…   ok
   31  mutation.applied       m1   bf94e503c1c002f277248b14b6afd1910bb8ce6f
   32  expression.restart_requested m1   apply bf94e503… seed . fitness docs_coverage +
```

## Express

The applied genome is not yet the running expression. Three ways it
becomes one, chosen by what the organization and the manifest say:

**Under `hale dna dev`, the host.** It sees the restart request,
cuts a fresh artifact (`current.topology`, keeping the old one as
`previous.topology`), rebuilds, terminates the old process (SIGTERM,
then SIGKILL after five seconds) and starts the new one with
`HALE_DNA_RESTART_FOR=<id>` and `HALE_DNA_EXPRESSION=<shape> build
<digest>`. The new expression journals `expression.restarted` itself,
at birth. If the candidate does not build, the host reports
`build_failed` on the membrane and the organization rolls back.

```text
hale dna dev: m1 requests a restart (apply bf94e503… seed . fitness docs_coverage +)
hale dna dev: expression restarted (pid 1875137) as 3c9b9327e480d349 build 7a489ad3e72c
```

**Under `hale dna run` with a fleet.** The host pushes the revision
to `refs/dna/revisions/<rev>` and appends `fleet.deploy` — the plan,
the revision, the seed, the instances touched (every instance with a
node whose seed is that directory), the reason. Every node answers
the row for its own instances and reports `instance.up`. The seed
that changed decides what is touched: a change to `services/api`
redeploys the API's replicas and leaves the worker running.

**Through a deployment gateway.** With `ShellDeployment { command,
seed }` in the organization, the substrate runs `<command> express
<candidate> <seed>` in its own handler; the command owns expressing
and judging, its exit code is the observation, and the record says
`expression.deployed`. No host is asked.

A request for the organization's own seed (`dna/org`) is answered
by the host under both verbs: the org chart changed, so the
organization is rebuilt and restarted, and watched.

## The observation window

`--observe <secs>` (default 15) is how long the new expression must
stay up. At the end the host publishes `ExpressionObserved` on the
membrane — `healthy` with the shape hash it observed, or what went
wrong — and the organization decides, against the Mutation's state:

- **`healthy`**: `mutation.retained`, the worktree dissolved, and the
  originating pressure re-measured (`pressure.remeasured`, naming the
  Task and the fitness signals);
- **anything else**: the genome is rolled back to the Mutation's base
  (`git reset --keep`, through the gateway, `mutation.rolled_back`)
  and the old expression is asked back — a restart request, or a
  deploy row, marked `rollback`, which is answered without opening
  another window.

Only an applied Mutation is judged; a late or repeated report changes
nothing.

```text
hale dna dev: m1 observed healthy for 5s as 3c9b9327e480d349
   34  expression.observed    m1   healthy 3c9b9327e480d349 up for 5s
   35  pressure.remeasured    m1   task t1 fitness docs_coverage +: healthy up for 5s
   36  mutation.retained      m1   observed healthy as 3c9b9327e480d349
```

On a fleet the window is over every touched instance: each must
report `instance.up` at the revision (three minutes to settle), then
none may report `instance.exited` for the window. The shape in the
observation is the one the instances reported.

## When the expression crashes

Under `dev`, if the new expression exits inside the window, the host
reports `crashed` with the exit code; the organization rolls back and
asks for the old expression, which the host restarts. If the
*organization* exits inside its own window there is nobody left to
decide, so the host accounts for it explicitly: it appends
`expression.crashed` and `mutation.rolled_back` itself, resets the
genome to the base, and restarts the base organization.

On a fleet, one instance exiting is `expression.crashed` naming the
instance and its node — `instance api-1 on edge-2 exited 137 in the
observation window` — and the rollback deploy row touches every
instance the apply did, so both replicas come back at the base. An
instance that never came up is `never expressed by …` and
`build_failed`.

## What this does not do

Live state migration. A restart is a restart: the old process ends,
the new one starts from its constructors. An application that must
carry state across a change carries it the way it would across any
deploy. Mutations that alter live state layout are a different
problem (`reperspective` covers the footprint-preserving case; the
rest is future work), and DNA does not pretend otherwise.
