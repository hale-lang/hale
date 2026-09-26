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
hale dna work run --as position:agent [--wait <secs>]
```

`--api` names the head (default `HALE_DNA_API`, else
`http://127.0.0.1:8793`, the API child of `dna/face/start.sh`). The
verbs land on the command table: `next` is `dna.attempt.claim`,
`renew` is `dna.attempt.renew`, `submit` is `dna.attempt.outcome`,
`settle` reads that command back, `release` is `dna.attempt.release`,
and `friction` is `dna.friction.file`.

## The cycle

**`next`** claims the next admitted, outstanding attempt of the
position's kind that fits what the leg has (`--capabilities`, by
default what the kind requires), may see (`--classes`, by default
`public internal`; naming none is naming no class) and works for
(`--orgs`), for `--ttl` seconds (600). The answer is the lease as a
value: the attempt, its Work and task, the holder and token, until
when. Nothing fitting is `state: refused` with the reason. Asked
again by the same position it is the same lease.

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
attempt is another leg's to claim; `friction` files what got in the
way as a row nobody admits.

**`run`** is one cycle through the project's performers: claim,
brief, perform, submit, settle.

## The performers

`dna/org/work.hl`, generated at init beside the catalog, names one
performer of each kind:

- a **person**: the brief is rendered as text and the outcome is
  theirs to `submit`;
- a **deterministic** performer: a program of the project's own. Give
  it the work kinds it takes and it wins for them
  (`legs::FixedAnswer { kinds: "agent", result: "…" }` is the
  smallest one; `NoDeterministic` takes nothing);
- a **model**: the model leg, which the next PR brings; until then
  `NoModel` refuses and the work is a person's.

A performer is handed a brief and the hands it may use — git in a
scratch worktree (never the primary checkout), the forge through
`gh` (the forge decides, a leg never merges), this toolchain; deploy
and the heart's API refuse until GH #987 hands them over — and answers
with a performance: the disposition and result, the calls it made,
the receipts to file, the digest of what it was shown.

## Through `hale mcp`

The `hale_dna_work` tool takes a verb and its flags as given on the
command line, and answers with the same JSON.
