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
verbs land on the head's gated topics: `next` is `AttemptClaim`,
`renew` is `AttemptRenew`, `submit` is `AttemptOutcome`, `settle`
reads that call back, `release` is `AttemptRelease`, and `friction` is
`FrictionFile` — all gated `position`, and listed to the leg by `hale
describe`.

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
would take to a person.

## Worker mode: `loop`

`hale dna work loop --as position:agent --parallel N` is a worker: `N`
child processes, each this program run once (`run`), each its own
holder — `position:agent#1` … `position:agent#N`, so two workers of
one position never share a lease — supervised: a child that ends is
started again, after a pause (`--idle-ms`, 2000) when it found nothing
to claim. Every child's answer is printed as it ends, as a line
naming the worker and its holder, and the loop's own line when it
ends. `--once` runs each child once; `--only <kind>` claims one work
kind; `--performer`, `--capabilities`, `--classes`, `--orgs` and
`--ttl` pass through to the children. `--parallel` is 1..64.

`hale dna work loop --drain`, on its own, tells the loop running in
this project to take no new work and end once its children have: it
leaves a marker beside the leg (`.hale/dna/legs/drain`), which the
loop reads between tasks and removes when it starts. Nothing is held
between tasks: a worker killed mid-task leaves a lease that expires,
and the owner asks again.

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
  `agent`, `service` and `software` kinds (`kinds`); `NoModel` takes
  nothing.

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
tasks. The brief is rendered as a prompt (`--render prompt`), sent
through the router with the Work's data class, knowledge bindings,
tool grant and cost ceiling from the hat, and the answer is the
result. Every call the router answers is **evidence**: the backend,
the model it reported, the tokens, the cost, the wall time, under the
prompt and context digests, handed back with the outcome; the owner
journals it as `model.called` rows on the attempt, so tokens per task
hold out of process as they do in it. The prompt as sent is filed as
the attempt's receipt when its class allows (`public`, `internal`).

A **rate-limited** call (HTTP 429, or a backend saying so) is backed
off inside the attempt: up to `retries` (3) more tries, the first
after `backoff_ms` (1000), each wait double the last, the lease held
meanwhile. Each wait is a row of evidence of its own — `refused:
rate limited, backed off 1000ms: …` — so the attempt's cost in time
sits in the record beside its cost in tokens, and the narrative says
what it waited. A call still refused after the retries is a `failed`
outcome, with the reason.

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
