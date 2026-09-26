# Legs: `hale dna work`

A leg is what performs a Work: a person, a program, a model. It holds
nothing between tasks and has no database role. Everything it knows
it reads from the head's API, and everything it does is a command on
the head's table: it claims an attempt, reads the hat, works, hands
the outcome back under the lease it was given, and lets go. The owner
still admits and settles.

`hale dna work` is a leg, as verbs. Each verb is an API client that
prints one JSON object and holds nothing afterwards; a verb run twice
under the same lease is one act, and the second run reads the first's
receipt. The position a leg works as is the graph's `position:<name>`
id, never a free string.

```text
hale dna work next --as position:agent          claim the next attempt for a position
hale dna work brief --attempt <id> [--render text|prompt|agent] [--plain]
hale dna work renew --as … --attempt <id> --token <n> [--ttl 900]
hale dna work submit --as … --attempt <id> --token <n> --result <text> [--result-file f]
                     [--narrative …] [--evidence-file calls.json] [--receipt-file f --receipt-class internal]
                     [--hat-digest … --hat-head … --hat-watermark n --prompt-digest …]
hale dna work settle --attempt <id> --token <n> [--wait <secs>]
hale dna work release --as … --attempt <id> --token <n> --why <text>
hale dna work friction --as position:agent [--attempt <id>] --text <what got in the way>
hale dna work run --as position:agent [--performer person|deterministic|model] [--wait <secs>]
hale dna work loop --as position:agent [--parallel N] [--once] [--only <kind>] [--performer …]
hale dna work loop --drain
```

`--api` names the head (default `HALE_DNA_API`, else
`http://127.0.0.1:8793`, the API child of `dna/face/start.sh`). The
verbs land on the command table: `next` is `dna.attempt.claim`,
`renew` is `dna.attempt.renew`, `submit` is `dna.attempt.outcome`,
`settle` reads that command back, `release` is `dna.attempt.release`,
and `friction` is `dna.friction.file`.

## Getting started: the whole loop on a fresh project

One machine, a fresh project, the fake backend, no key: a judgment is
asked for, a person performs one as a leg, a model performs the next
in worker mode, and the record shows what each cost. Every command
below was run as written; what it printed is abridged to the lines
that matter.

**A project, with the fake behind the model leg.** `hale dna new`
under `HALE_DNA_DISCOVER=off` finds no backend, so the generated
performers hand agent work to a person (`NoModel`). It also seats the
record: the uid of whoever made it is mapped to them in the record's
local config, and the record declares `dna.trust = local` — one
person holding every authority — so the head's socket knows that peer
and they hold every position; the verbs are theirs to call:

```text
$ HALE_DNA_DISCOVER=off hale dna new demo && cd demo
seated  the head's socket knows uid 1000 as riley (dna.unix.member); the record declares dna.trust = local, where they hold every position
```

For a dry run, give the agent router the fake and put the catalog
behind the leg:

In `dna/org/models.hl`, point `agent_models()` at a fake:

```hale,fragment
fn fake() -> dna::FakeModel {
    return dna::FakeModel { name: "fake", model: "fake-1", answer: "assessment: the migration is safe to ship; the rollback path is exercised by the fixture" };
}
fn agent_models() -> dna::ModelRouter {
    return dna::ModelRouter { quick: fake(), deep: fake(), private: fake() };
}
```

and in `dna/org/work.hl`, replace the generated `model()` with the
one its comment shows:

```hale,fragment
fn model() -> legs::ModelPerformer {
    return legs::ModelPerformer { router: agent_models() };
}
```

`hale check dna/org` says `ok`; commit both. The organization a fresh
project generates already hands its agent work to legs
(`agent: dna::LegRelay { name: "legs" }` in `dna/org/main.hl`).

**The organism and the head.** The nerves first (`nats-server -js`
on the loopback, or the one `dna/compose.yaml` brings up), then the
organism under `hale dna run` with the two lines `nerves migrate`
prints, then the head — the API a leg talks to — with the project
attached:

```text
$ HALE_DNA_NATS_URL_OWNER=nats://127.0.0.1:4222 hale dna nerves migrate
HALE_DNA_NATS_ORG=dna_8758…
HALE_DNA_NATS_URL_SPINE=nats://spine:…@127.0.0.1:4222
$ HALE_DNA_NATS_ORG=… HALE_DNA_NATS_URL_SPINE=… hale dna run . --no-iris
hale dna run: organization (pid 2704661) from …/demo under LOTUS_OBS=1
hale dna run: the organization reads its facts from the nerves (DNA_8758…)
$ dna/face/start.sh …/demo          # from a checkout of hale; the API child listens on 8793
```

The API child is the head the verbs talk to: its reads over HTTP,
its commands on the socket its capabilities name
(`$XDG_RUNTIME_DIR/hale/dna/<record id>.sock`), which is where the
leg finds it.

**A judgment asked for.** A change is the leader's to classify; an
assessment is the asker's word, and is admitted as one judgment leaf
that capability-first routing hands to an agent — the legs' relay,
which answers pending for a leg:

```text
$ hale dna task create --judgment assess whether the storage migration is safe to ship
task t1 born for intent i319463c4 [active]
$ hale dna history
   40  workflow.admitted      t1                {"definition": "ask-judge", …}
   44  attempt.admitted       t1/wf1/s0/j/a0    …
   45  effect.requested       attempt:t1/wf1/s0/j/a0   attempt t1/wf1/s0/j/a0 by agent
```

**A person performs it.** Claim it, read the brief, hand the outcome
back under the lease, and read the settlement:

```text
$ hale dna work next --as position:agent
{"verb": "next", "state": "succeeded", "attempt": {"state": "claimed", "attempt_id": "t1/wf1/s0/j/a0",
 "holder": "position:agent", "token": 1, "until": 1790433675, …}}
$ hale dna work brief --attempt t1/wf1/s0/j/a0 --render text --plain
BRIEF t1/wf1/s0/j (attempt t1/wf1/s0/j/a0) of task t1
position: position:agent (agent)
objective: assess whether the storage migration is safe to ship
output contract: Assessment
data class: internal
requires: judgment
…
head acae4170… watermark -1 hat sha256:c2d70c78… legs-render/1/text
$ hale dna work submit --as position:agent --attempt t1/wf1/s0/j/a0 --token 1 \
      --result "assessment: safe to ship; the migration is additive and the rollback restores the previous schema"
{"verb": "submit", "state": "succeeded", "attempt": {"state": "requested", …}}
$ hale dna work settle --attempt t1/wf1/s0/j/a0 --token 1 --wait 30
{"verb": "settle", "state": "succeeded", "attempt": {"state": "settled", "disposition": "done", …}}
```

The node beside the organism relayed the outcome onto the nerves, the
organism settled the attempt and the workflow, and the record has the
rows: `attempt.claimed`, `attempt.outcome_requested`,
`attempt.outcome`, `workflow.settled`.

**A model performs the next, in worker mode.** Ask again, and start a
worker that runs one task per child and ends:

```text
$ hale dna task create --judgment assess whether the retry policy of the ingest path bounds its queue
task t2 born for intent i3194efb4 [active]
$ hale dna work loop --as position:agent --parallel 1 --once --wait 30
{"verb": "loop", "worker": 1, "holder": "position:agent#1", "ran": {"verb": "run", "performer": "model",
 "attempt_id": "t2/wf1/s0/j/a0", "token": 1, "hat_digest": "sha256:83a4a629…", "prompt_digest": "sha256:74dbbc3e…",
 "renderer": "legs-render/1", "state": "settled", "request_id": "work-submit:t2/wf1/s0/j/a0:1"}}
{"verb": "loop", "state": "ended", "workers": 1, "ran": 1, "drained": false}
```

The child claimed as `position:agent#1` (its `--worker 1`), rendered
the hat as a prompt, sent the render alone to the fake through the
catalog's router, handed the answer back with the call as evidence,
and waited for the settlement. Without `--once` the loop keeps going: a child
that performed is started again at once, one that found nothing waits
its backoff, and `hale dna work loop --drain` (or SIGTERM) ends it.

**What it cost.** The record answers per task, from the evidence the
leg handed back:

```text
$ hale dna history t2
usage of t2: 1 call(s) · 0 in · 22 out · 10 µ$
       by position: position:agent 1 call(s) · 0 in · 22 out · 10 µ$
       by backend: fake/fake-1 1 call(s) · 0 in · 22 out · 10 µ$
   63  model.called           t2/wf1/s0/j/a0    {"adapter": "legs-router", "backend": "fake", "reported_model": "fake-1", …}
   64  attempt.outcome        t2/wf1/s0/j/a0    {"disposition": "done", …}
   68  workflow.settled       t2                {"disposition": "done", …}
$ hale dna status
tasks:      2
  t1 [done] i319463c4: ask-judge@1 (workflow)
  t2 [done] i3194efb4: ask-judge@1 (workflow)
```

A person's task shows no usage: nothing was called. Put a real
backend back in `agent_models()` and the same loop runs against it,
with the credential fetched per task and every call's tokens and
cost on the attempt.

## Positions, ids, exit codes

`--as` and `--holder` are positions: `position:<name>` is the graph's
id, and `position:<name>#<n>` one worker of several. The head admits a
position the graph names (a `graph.node` row) or one of the
organization's own (`position:leader`, `position:editor`,
`position:agent`, `position:human`, `position:service`,
`position:software`); anything else is refused with the reason.

Request ids: `next` mints a fresh id per call (a claim by its holder
renews in the store, so a second `next` is the same lease); `submit`
and `release` are `work-submit:<attempt>:<token>` and
`work-release:<attempt>:<token>`, so a repeat is one act; `renew` is
counted by `--renewal <n>` (1 by default: say `2`, `3`, … for the
next), never by the clock; `friction` keys on the text and the
attempt. `--request-id` overrides any of them.

Exit codes: **0** the head admitted it (or the read answered); **1** a
refusal — the receipt is printed, with `state: refused` and the reason
— or a head that could not be reached; **2** a usage error, judged
before the head is asked (a missing flag, a free-string position, a
number that is not one).

The leg is built once per project under `.hale/dna/legs/<key>`, the
key being what it is built from (the performers, the catalog, the
vendored seed); a change builds it again, two legs at once build apart.

## The cycle

**`next`** claims the next admitted, outstanding attempt of the
position's kind that fits what the leg has (`--capabilities`, by
default what the kind requires), may see (`--classes`, by default
`public internal`; naming none is naming no class) and works for
(`--orgs`), for `--ttl` seconds (600). The answer is the lease as a
value: the attempt, its Work and task, the holder and token, until
when. Nothing fitting is `state: refused` with the reason (exit 1);
an attempt whose outcome is handed back and awaits the owner awaits
nobody else. Asked again by the same position it is the same lease.

**`brief`** reads the hat: the position and its charter, the
practices as structure, the bindings, the grant, the contract, the
class, the Work's history, the head and watermark it was rendered at,
and its digest. `--render text` is a person's brief, `prompt` what a
model is sent, `agent` the prompt with the hands; the answer carries
the hat digest, the digest of what was rendered and the renderer's
version (`legs-render/1`), which the outcome records so replay renders
from the recorded hat. `--plain` prints the rendering alone.

**`submit`** hands the outcome back under the lease (`--disposition`
done, failed, declined or timeout), with the calls made as evidence
and the receipts to file, and the hat it wore. `settle` reads whether
the owner settled it (`--wait` polls). `renew` extends the lease and
keeps the token; `release` gives it back without an outcome, and the
attempt is another leg's to claim (under a new token); neither is
admitted while an outcome under the lease awaits the owner. `friction`
files what got in the way as a row nobody admits. `brief` answers in
the same envelope as the rest (`verb`, `state`, and the `hat`).

**`run`** is one cycle through the project's performers: claim,
brief, perform, submit, settle. `--performer` names the performer to
be, instead of the catalog's choice: `person` leaves a Work the model
would take to a person; a performer that does not take the kind
refuses, forced or not, and the lease is given back. An outcome the
head refuses (the lease expired meanwhile) is printed as its receipt,
exit 1.

## Worker mode: `loop`

`hale dna work loop --as position:agent --parallel N` is a worker: `N`
child processes, each this program run once (`run`), each its own
holder — `position:agent#1` … `position:agent#N`, so two workers of
one position never share a lease — supervised: a child that ends
after a task is started again at once; one that found nothing to
claim, left the Work to a person, or gave it back waits its slot's
backoff first (`--idle-ms`, 2000, doubling per idle run in a row up
to a minute), so a loop with nothing to do never writes the record in
a tight circle. Each child's answer is printed as one JSON line as it
ends, its output read as it runs; the loop's own line comes last.
`--once` runs each child once; `--only <kind>` claims one work kind;
`--performer`, `--capabilities`, `--classes`, `--orgs` and `--ttl`
pass through to the children, which each mint their own claim id.
`--parallel` is 1..64 and `--performer` one of the three, judged
before the head is asked.

The loop owns its process tree. The leg is its program's main locus,
so SIGTERM or SIGINT drains it: no child is started again, every
running child is told to end and, after three seconds, made to, and
the loop ends once each is reaped — nothing of it is left running. A
child ended mid-task leaves a lease that expires, and the owner asks
again. `hale dna work loop --drain`, on its own, tells the loop
running in this project to take no new work and end once its
children have finished: it leaves a marker beside the leg
(`.hale/dna/legs/drain`), which the loop reads between ticks and
removes when it starts.

Through `hale mcp` a loop runs only with `--once` (or `--drain`): an
unbounded loop would hold the server, and belongs to a terminal.

## The performers

`dna/org/work.hl`, generated at init beside the catalog, names one
performer of each kind:

- a **person**: the brief is rendered as text and the outcome is
  theirs to `submit`;
- a **deterministic** performer: a program of the project's own. Give
  it the work kinds it takes and it wins for them
  (`legs::FixedAnswer { kinds: "agent", result: "…" }` is the
  smallest one; `NoDeterministic` takes nothing);
- a **model**: the model leg — the catalog's agent router
  (`agent_models()` from `dna/org/models.hl`) behind the performer,
  `legs::ModelPerformer { router: agent_models() }`. It takes the
  `agent`, `service` and `software` kinds (`kinds`). A project
  initialised with no backend configured gets `NoModel`, which takes
  nothing, so its agent Works are a person's rather than attempts
  burnt as failed; the generated file says how to put the catalog
  behind the leg once a backend is there.

A performer is handed a brief and the hands it may use — git in a
scratch worktree (never the primary checkout), the forge through
`gh` (the forge decides, a leg never merges), this toolchain; deploy
and the heart's API refuse until GH #987 hands them over — and answers
with a performance: the disposition and result, the calls it made,
the receipts to file, the digest of what it was shown.

## The model leg

The model performer runs the catalog — the router, its adapters (an
OpenAI-shaped endpoint, Anthropic, a local model, a harness under
confinement, the fakes) and the tape (`RecordedModel`, wrapping any of
them) — out of process, one task per run, with nothing held between
tasks. The brief is rendered as a prompt (`--render prompt`) and that
render alone is what the model is sent, with the Work's data class,
knowledge bindings, tool grant and cost ceiling from the hat: the hat
is structure, the leg renders it, and the hat's digest is the context
digest on every row of evidence (the tape's key). The answer is the
result. Every call the router answers is **evidence**: the backend,
the model it reported, the input and output tokens as the backend
reported them, the cost, the wall time, under the prompt and context
digests, handed back with the outcome; the owner journals it as
`model.called` rows on the attempt, so tokens per task hold out of
process as they do in it. The prompt as sent is filed as the attempt's
receipt when its class allows (`public`, `internal`).

A **rate-limited** call (HTTP 429, or a backend saying so) is backed
off inside the attempt: up to `retries` (3) more tries, the first
after `backoff_ms` (1000), each wait double the last. Before each
wait the lease is renewed through the head for the wait and a margin,
so the Work is not lost to another leg meanwhile, and each wait is a
row of evidence of its own — `refused: rate limited, backed off
1000ms: …; lease renewed` — so the attempt's cost in time sits in the
record beside its cost in tokens, and the narrative says what it
waited. A call still refused after the retries is a `failed` outcome,
with the reason.

`hale dna models`, the probe, stays a host verb: it asks each backend
of the catalog one small request in process. The catalog and the tape
stay in the core (`models.hl`, `tape.hl`) while the owner's editor
and leader call them in process; they move when their stages land.

## An external harness

A harness of your own plugs in with the verbs, and needs no performer
in `work.hl`: `next` claims, `brief --render agent` is the prompt with
the hands, the harness does the work, `submit --evidence-file
calls.json` hands it back with its calls as evidence (the array of
`model.called` bodies) and the digests the brief reported. Through
`hale mcp` the same verbs are one tool, `hale_dna_work`.

## Through `hale mcp`

The `hale_dna_work` tool takes a verb and its flags as given on the
command line, and answers with the same JSON.
