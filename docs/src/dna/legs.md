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
hale dna work next --as position:agent [--effect <class>]   claim the next attempt for a position
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
verbs land on the command table: `next` is `dna.attempt.claim`,
`renew` is `dna.attempt.renew`, `submit` is `dna.attempt.outcome`,
`settle` reads that command back, `release` is `dna.attempt.release`,
and `friction` is `dna.friction.file`.

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

## Through `hale mcp`

The `hale_dna_work` tool takes a verb and its flags as given on the
command line, and answers with the same JSON.
