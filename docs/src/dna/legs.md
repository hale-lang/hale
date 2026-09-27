# Legs: `hale dna work`

A leg is what performs a Work: a person, a program, a model. It holds
nothing between tasks and has no database role. Everything it knows
it reads from the head's API, and everything it does is a call on
the head's gated topics: it claims an attempt, reads the hat, works, hands
the outcome back under the lease it was given, and lets go. The owner
still admits and settles.

`hale dna work` is a leg, as verbs. Each verb is an API client that
prints one JSON object and holds nothing afterwards; a verb run twice
under the same lease is one act, and the second run reads the first's
receipt. The position a leg works as is the graph's `position:<name>`
id, never a free string.

```text
hale dna work next --as position:agent [--worker n] [--effect <class>]   claim the next attempt for a position
hale dna work brief --attempt <id> [--render text|prompt|agent] [--plain]
hale dna work renew --as … --attempt <id> --token <n> [--ttl 900]
hale dna work submit --as … --attempt <id> --token <n> --result <text> [--result-file f] [--effect <class>]
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
verbs land on the head's gated topics: `next` is `AttemptClaim`,
`renew` is `AttemptRenew`, `submit` is `AttemptOutcome`, `settle`
reads that call back, `release` is `AttemptRelease`, and `friction` is
`FrictionFile` — all gated `position`, and listed to the leg by `hale
describe`.

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

`--as` is a position: `position:<name>`, the graph's id. The head
admits a position the graph names (a `graph.node` row) or one of the
organization's own (`position:leader`, `position:editor`,
`position:agent`, `position:human`, `position:service`,
`position:software`); anything else is refused with the reason. The
lease's holder is the position a leg works as; a worker of a loop is
`--worker <n>`, held as `position:<name>#<n>`, so two workers of one
position never share a lease. Who the leg *is* comes from the socket:
the peer's credentials, mapped to a person by the record
(`git config --local --add dna.unix.member "uid:<n>=<person>"`), and
the verbs are listed to a peer whose person holds a position.

Request ids: `next` mints a fresh id per call (a claim by its holder
renews in the store, so a second `next` is the same lease); `submit`
and `release` are `work-submit:<attempt>:<token>` and
`work-release:<attempt>:<token>`, so a repeat is one act; `renew` is
counted by `--renewal <n>` (1 by default: say `2`, `3`, … for the
next), never by the clock; `friction` keys on the text and the
attempt. `--request-id` overrides any of them.

Exit codes: **0** the head admitted it (or the read answered); **1** a
refusal — the receipt is printed, with `state: refused` and the reason
— a verb this peer may not call (`unknown`: outside the caller's
slice), or a head that could not be reached; **2** a usage error,
judged before the head is asked (a missing flag, a free-string
position, a number that is not one).

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
performer of each kind, and every one declares its **effect class**
(`effect`): what a settle that failed may have left behind.
`effect_free` answered and touched nothing; `idempotent` may be run
again to the same end; `uncertain` — an agent's seat with tools — may
have acted once already. There is no default: a catalog with a
performer that declares none is refused by the verbs that claim or
hand back (`next`, `submit`, `run`, `loop`; exit 2, naming it), and
read by the rest.

- a **person** (`legs::Person { effect: "idempotent" }`: asked again,
  they answer again): the brief is rendered as text and the outcome is
  theirs to `submit`;
- a **deterministic** performer: a program of the project's own. Give
  it the work kinds it takes and it wins for them
  (`legs::FixedAnswer { kinds: "agent", effect: "effect_free", result:
  "…" }` is the smallest one; `NoDeterministic` takes nothing);
- a **model**: the model leg — the catalog's agent router
  (`agent_models()` from `dna/org/models.hl`) behind the performer,
  `legs::ModelPerformer { router: agent_models(), effect:
  "effect_free" }` for hosted backends that answer, `effect:
  "uncertain"` when a tier is a harness with tools (init writes the
  class the catalog it found calls for). It takes the
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

The class rides with the work. On the **claim** it is the filter,
and the hat says what the Work admits (`effects`). The class decides
what a failed settle becomes, never what the performer may do — that
is the hat's tool grant — so a Work that is answered (a judgment, an
analysis, a chore) admits every class, and an edit, which changes
source, admits no `effect_free` performer. `run` and `loop` claim
with the class of the performer they chose; `next` and `submit`
claim and answer with the person's, or the one `--effect` names
(the flag is theirs alone). The claim row records the class, a
renewal carries it on, and an outcome under the lease names the
same class or is refused. On the **outcome** it is evidence: the row
the owner journals carries `effect_class`.

And it decides what a failed settle becomes. On an `effect_free` or
`idempotent` performer a settle that fails — the answer to the
submit was lost and the outcome cannot be read back, the outcome was
refused, or the owner refused it — is `unsettled` (exit 1): the lease
is given back when it was still the leg's, the owner asks again under
a fresh token, and the loop runs the slot again after a rest, which
performs anew (never a second submit under the same request). On an
`uncertain` performer nothing is retried by a program. The leg
**marks the attempt unresolved** at the head — an outcome of
disposition `unresolved`, carrying its calls as evidence and heard
past the lease's end, which the head records as `attempt.unresolved`
and as `effect.result unknown` on the attempt — files **friction**
naming the attempt, the holder, the lease and the way out, answers
`unresolved` (exit 1), and the loop stops that slot. Marked, the
attempt is nobody's to claim: not another leg's, not its own
holder's, not a loop's. A person decides:

```sh
hale dna effect resolve attempt:<id> --outcome ok|failed
```

and the owner settles the attempt on that word (`failed` spends the
attempt, and the Work is asked again if its allowance has more).

The same holds when no word comes at all. An `uncertain` claim whose
lease lapses with no outcome — the leg died mid-session — is never
asked of a leg again: the owner records the effect unknown itself,
and waits for the same resolution. So a claim of that class is taken
for an hour by default (`--ttl`), a session with tools being no
ten-minute affair.

The model leg applies the rule by what the backend says of the call.
Every model result says whether the backend **may have acted**
(`made`): a refusal before anything was sent or run — no credential,
a data class, a harness not on PATH, a connection never opened, a
4xx that did no work — did not; anything after did. Behind an
`uncertain` performer a refusal that may have acted is `unresolved`,
never retried, never failed over; behind the other two it is a
`failed` outcome. A rate-limited call is asked again after a wait
only when it was never made: a harness session cut short by a limit
is not a call to repeat.

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

## Over the head's socket

The head's commands are its gated topics on its api socket (GH #1104
piece 5): one JSON object per line, each verb a `call` on its topic
with the payload the description gives it — `AttemptClaim`,
`AttemptRenew`, `AttemptOutcome`, `AttemptRelease`, `FrictionFile`,
all gated `position`, and `CommandLookup`, which `settle` reads a
command back with — and the receipt on the value channel, exactly as
the record wrote it. The reads stay HTTP: the record head from
`/applications`, the hat from `/dna/context`, and the socket's path
from `/capabilities` (`api.socket`), which is where the leg finds it;
`--socket` or `HALE_DNA_SOCKET` name it outright. One socket per
record, under `$XDG_RUNTIME_DIR/hale/dna/`; `LOTUS_API` overrides it
where the head runs.

The socket authenticates: the principal is the peer's credentials, as
the kernel vouches for them, and the record maps them to a person
(`dna.unix.member`, in the record's local config; `hale dna new` maps
its maker). A record that declares `dna.trust = local` — as the one
`hale dna new` makes does — is one person's: whoever a peer maps to
holds every position; a record that declares nothing, or any other
trust, leaves it to the graph's `holds` edges. A leg is not asked who it is —
there is no holder flag — and a peer whose person holds no position
sees no verb at all: the leg says so at attach, before it asks
anything. `hale describe <socket>` lists what a peer may call;
`hale call <socket> AttemptClaim '{…}'` is the same claim by hand.

## Dogfood: the loop on voice

`dna/tests/dogfood_voice_test.hl` runs the legs' loop on the vendored
voice repository, through the head's socket, with a deterministic
performer standing in for the model. The processes it stands up:

| Process | What runs | What it reads |
|---|---|---|
| memory | Postgres, the record's spine and head roles (`hale dna memory migrate`) | `HALE_DNA_MEMORY_DSN_OWNER`; only the route needs it |
| head | `dna/api/practice_review`, its commands on the record's socket, its reads on HTTP | the record, and the policy `HALE_DNA_COMMAND_POLICY` names |
| owner | the organization, relaying software work to legs | the record |
| leg | `hale dna work run`, the project's `dna/org/work.hl` | the head (`HALE_DNA_API`), the socket it names |

The owner is the one part no generated organization is yet: the one
`hale dna init` writes relays no work to legs. The fixture runs an
owner in its own process, wired as a project would wire its
`dna/org/main.hl`, with a workflow whose one Work (`output_contract:
"Patch"`, no requirement, so the software kind) is asked of it:

```text
work: dna::WorkSystem {
    software: dna::LegRelay { name: "legs", performer_kind: "software" },
    software_reconciler: dna::RelayReplay { }
}
```

By hand, the project and its seat (memory needs
`HALE_DNA_MEMORY_DSN_OWNER`, or the project's `dna/compose.yaml`):

```sh
cp -r "$HALE_SRC/dna/tests/onboarding/voice" ~/voice && cd ~/voice
git init -q -b main && git config user.name riley && git config user.email riley@local
git add -A && git commit -qm voice
hale dna init .
hale dna memory migrate .            # prints the spine and head DSNs
hale dna dev . --no-iris &           # the organization, which ratifies and fills
hale dna review holes approve --as ada --authority board
hale dna fill api/dev riley --as ada # a Board Review in ada's name
hale dna review <its id> approve --as grace --authority board
git config --local --unset-all dna.trust   # init seated its maker under local trust
git config --local --replace-all dna.unix.member "uid:$(id -u)=riley"
```

(The fixture writes the ratifications and the `holds` edge as rows
instead, as `graph_route_test` does. The proposer of a holder may not
ratify it, so a second person decides it by hand.) `hale dna init`
seats whoever runs it — this uid mapped to `$USER`, and `dna.trust =
local`, where that person holds every position — so the run takes the
trust back: with no `dna.trust` declared, the graph's `holds` edges say
who holds what. Until
riley holds a position the head's socket lists no claim to the peer
(`hale describe <socket>`), and the leg says so at attach; once riley
holds `api/dev` the leg claims as that peer, the lease in riley's name.

The head runs from the toolchain's source, with a policy naming who
may recover a lease, and without the spine's role:

```sh
cd "$HALE_SRC" && hale build dna/api/practice_review
mkdir -p ~/voice/.hale/dna
printf '{"format":"dna.practice-review-authority/1","application_id":"%s","grants":[{"mode":"local","name":"riley","authority":"board","practice_propose":false,"review_verdict":false,"recover":true}]}' \
    "$(git -C ~/voice rev-list --max-parents=0 refs/dna/journal)" > ~/voice/.hale/dna/authority.json
env -u HALE_DNA_MEMORY_DSN_SPINE -u HALE_DNA_MEMORY_DSN_OWNER \
    HALE_DNA_COMMAND_POLICY=~/voice/.hale/dna/authority.json \
    "$HALE_SRC/dna/api/practice_review/practice_review" ~/voice 8793 &
cd ~/voice && HALE_DNA_API=http://127.0.0.1:8793 \
    hale dna work run --as position:api/dev --kind software --worker 1
```

The performer (`dna/tests/dogfood/work.hl.txt`, installed as the
project's `dna/org/work.hl`) takes the software kind, makes its change
in a scratch clone, commits it, pushes the commit to
`refs/legs/candidates/<attempt>` (the leg's namespace, never the
record's `refs/dna/`), and hands it back as `result_ref:
commit:<sha>` with the commit's patch as a receipt. Run again under a
new lease it makes the same change and moves the ref to it, which is
what makes it `idempotent`. Who must sign the change is the graph's
word:

```sh
hale dna route --diff HEAD..<sha>     # api's reviewer, for a change under api/
hale dna route Dockerfile             # nobody: today, the Review's own authority
```

A leg killed mid-task, its clone made, leaves its lease to lapse; the
owner asks again, and the next worker claims the attempt under the
next token and finishes it.

What does not exist yet is not stood in: the candidate becoming a
Review (GH #1156); that Review going to the graph's signers, a
non-signer's verdict refused, and a change nobody signs refused rather
than left to the Review's own authority (GH #1157); and the lease's
position bound to a position the peer's person holds (GH #1162) — the
gate today is "holds any live position".

## Through `hale mcp`

The `hale_dna_work` tool takes a verb and its flags as given on the
command line, and answers with the same JSON.
