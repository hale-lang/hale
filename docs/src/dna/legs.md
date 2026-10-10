# Legs, hands and voice

A **leg** performs one unit of work for a position: it claims an
attempt, wears the attempt's **hat**, acts with the **hands** it is
given, speaks to models through the **voice**, and hands the outcome
back under the lease it was given. The project owns its legs (the
performers in `dna/org/work.hl`); the spine admits the claim and
settles the outcome. A leg fails by letting its lease lapse: the spine
asks again, or, for a performer that may already have acted, waits for
a person. A leg holds nothing between tasks, and never holds a database
role, a lease past its end, or the right to settle its own work.

| part | what it is | where it lives |
| --- | --- | --- |
| legs | `hale dna work`: the verbs, the performers, the worker loop | `dna/core/legs` (vendored as `vendor/dna/legs`), `dna/org/work.hl` |
| hat | one Work's context, as structure, read per task | `dna/core/hat.hl` |
| hands | what a performer acts with: git, the forge, the toolchain | `dna/core/legs/hands.hl` |
| voice | the model seam: the catalog, its backends, the tape | `dna/org/models.hl`, `dna/core/models.hl`, `dna/core/tape.hl` |

## One judgment, performed by hand

You need an organism running ([The heart and the body](./heart.md))
and a head serving its API ([The head and the face](./head.md)). The
verbs reach the head at `--api`, which defaults to `HALE_DNA_API`, else
`http://127.0.0.1:8793`. They read over HTTP, presenting
`HALE_DNA_ID_TOKEN` as the bearer where the head serves OIDC, and send
their commands over the socket the head names, where the kernel
vouches for who you are.

Where the head serves OIDC, get the token first. On the local stub
issuer (the one `dna/face/start.sh` starts), `login` mints it for the
person the record maps, with the client secret read from the vault
and never printed:

```sh
eval "$(hale dna work login --as alice)"   # export HALE_DNA_ID_TOKEN=…
```

A real issuer is refused: `login` prints the head's `/auth/login` URL
to sign in at in a browser, and mints nothing.

```sh
hale dna task create --judgment assess whether the storage migration is safe to ship
hale dna work next --as position:agent
hale dna work brief --attempt <attempt> --render text --plain
hale dna work submit --as position:agent --attempt <attempt> --token <n> --result "assessment: safe to ship"
hale dna work settle --attempt <attempt> --token <n> --wait 30
```

A judgment is admitted as one Work that the organization hands to
legs. `next` answers with the lease: the attempt, its Work and task,
the holder, the token, and `until`. `brief --render text --plain`
prints the hat written for a person. `submit` hands the result back
under the lease, and `settle` reads it back until the owner answers:
`settled` with the disposition, or refused with the reason. The record
now holds the attempt's rows, from `attempt.claimed` to
`attempt.outcome` ([One task, end to end](./workflow.md)).

The next judgment can go to a worker instead; `--once` runs each child
one task and ends:

```sh
hale dna work loop --as position:agent --parallel 1 --once
```

## The verbs

Every verb but `loop` prints one JSON object (`verb`, `state`,
`reason`, and the receipt's `attempt`) and holds nothing afterwards.
`submit` and `release` are keyed on the lease
(`work-submit:<attempt>:<token>`), so a verb run twice is one act and
the second run reads the first's receipt; `--request-id` overrides any
verb's id. Every verb takes `--api`.

| verb | what it does | its flags |
| --- | --- | --- |
| `login` | prints `export HALE_DNA_ID_TOKEN=…` for a head under the local stub issuer; the sign-in URL (and exit 1) for a real one | `--as <person>` (the record's only one when omitted), `--api` |
| `next` | claims the next attempt for a position (`Commands::claim`) | `--as`, `--kind`, `--capabilities`, `--classes` (default `public internal`), `--orgs`, `--ttl` (600), `--effect`, `--worker` |
| `brief` | reads the hat, or renders it | `--attempt` or `--work`, `--render text\|prompt\|agent`, `--plain` |
| `renew` | extends the lease and keeps the token (`AttemptRenew`) | `--as`, `--attempt`, `--token`, `--ttl`, `--renewal <n>` (1, 2, … per renewal) |
| `allowance` | asks the spine for the attempt's spend, and waits (`AttemptAllowance`) | `--as`, `--attempt`, `--token`, `--wait` (60) |
| `hand` | runs one of the Work's hands here: a file hand in the attempt's kept worktree, an infrastructure hand through the head | `--attempt`, `--name`, `--args` (JSON, as the hand's schema says) |
| `submit` | hands the outcome back under the lease (`AttemptOutcome`) | `--as`, `--attempt`, `--token`, `--from-worktree` (the kept worktree, validated and committed), `--result` or `--result-file`, `--disposition` (`done`), `--narrative`, `--evidence-file`, `--receipt-file`, `--receipt-class` (`internal`), `--effect`; the hat it wore: `--hat-digest`, `--hat-head`, `--hat-watermark`, `--prompt-digest`, `--renderer` |
| `settle` | reads the outcome back until the owner answers (`CommandLookup`) | `--attempt`, `--token`, `--wait` (0) |
| `release` | gives the lease back with no outcome (`AttemptRelease`) | `--as`, `--attempt`, `--token`, `--why` |
| `friction` | files what got in the way, a row nobody admits (`FrictionFile`) | `--as`, `--text`, `--attempt` |
| `run` | one cycle: claim, brief, perform, submit, settle | `--as`, `--kind`, `--performer person\|deterministic\|model`, `--wait` (30), and `next`'s filters |
| `loop` | a worker: supervised children, each a `run` | `--as`, `--parallel N` (1..64), `--once`, `--only <kind>`, `--idle-ms` (2000), `--performer`; `loop --drain` ends a running loop |

`--as` is the graph's id, `position:<name>`, never a free string, and
`/` is allowed (`position:api/dev`). It must be a position your person
holds: by the record's `holds` edges (a holder `hale dna fill` proposed
holds once the Board ratifies it), or any position under a record that
declares `dna.trust = local`, as `hale dna new` and `hale dna init`
declare. Who you are comes from the socket: your uid, mapped to a
person by `git config --local --add dna.unix.member "uid:<n>=<person>"`,
which `new` and `init` write for whoever runs them.

A claim matches attempts by performer kind, and `--kind` names it. The
kinds the organization hands to legs are `agent`, `software` and the
like; a position the graph states (`position:api/dev`) works as an
`agent`, and also takes the Works routed to it: a change asked with
`hale dna task create --to position:api/dev` is one `Patch` Work that
only that position's holder can claim, briefed in that position's hat,
and performed by the tool loop below. A `position` word in your
`--capabilities` is dropped: the head grants it from your seat alone. When nothing matches, the refusal names the kinds that
are outstanding for a leg and the flag that claims one.

Exit codes: **0** the head admitted it, or the read answered; **1** a
refusal (printed with `state: refused` and the reason), a verb outside
your slice, an unreachable head, or a `run` that ends `unsettled` or
`unresolved`; **2** a usage error, judged before the head is asked.

The leg is a program of the project's own, its performers over the
vendored legs seed, built once under `.hale/dna/legs/<key>`. `hale mcp`
exposes the verbs as `hale_dna_work`, and a Work's hands as
`hale_dna_hand`; over MCP a loop runs
only with `--once` or `--drain`.

## Performers and their effect classes

`dna/org/work.hl` names one performer of each kind. `init` writes it
beside the catalog; on a machine with `claude` on `PATH` its model and
its catalog read:

```hale,fragment
fn model() -> legs::ModelPerformer {
    return legs::ModelPerformer { router: agent_models(), effect: "uncertain" };
}

fn performers() -> legs::PerformerCatalog {
    // a person answers what they are asked; asked again, they answer again
    return legs::PerformerCatalog { person: legs::Person { effect: "idempotent" }, deterministic: deterministic(), model: model() };
}
```

With hosted backends and no harness the model's class is
`effect_free`. With no backend at all the model is `legs::NoModel { }`,
which takes nothing, so agent work goes to a person instead of burning
attempts; the deterministic performer is `legs::NoDeterministic { }`
until you write one (`legs::FixedAnswer { kinds: "agent", effect:
"effect_free", result: "…" }` is the smallest).

| performer | takes | briefed as | what it does |
| --- | --- | --- | --- |
| a person (`legs::Person`) | every kind | `text` | prints the brief; the outcome is theirs to `submit` |
| a deterministic one (`legs::FixedAnswer`, or your own) | the kinds it names, and it wins for them | `agent` | a program of the project's |
| the model (`legs::ModelPerformer`) | `agent`, `service`, `software` | `prompt` | the catalog's router, one task per run |

`run` picks the deterministic performer when it takes the kind, a
person for `human` work or when the model does not take the kind, and
the model otherwise. `--performer` forces one, but a performer never
answers a kind it does not take.

For a dry run with no key, put a scripted model behind the leg. Every
performer declares its class, the model included:

```hale
import "vendor/dna" as dna;
import "vendor/dna/legs" as legs;

fn fake() -> dna::FakeModel {
    return dna::FakeModel { name: "fake", model: "fake-1", answer: "assessment: the migration is additive and safe to ship" };
}

fn agent_models() -> dna::ModelRouter {
    return dna::ModelRouter { quick: fake(), deep: fake(), private: fake() };
}

fn model() -> legs::ModelPerformer {
    return legs::ModelPerformer { router: agent_models(), effect: "effect_free" };
}
```

**The effect class** says what a settle that failed may have left
behind. There is no default: a catalog with a performer that declares
none is refused by `next`, `submit`, `run` and `loop` (exit 2).

| class | means | a failed settle becomes |
| --- | --- | --- |
| `effect_free` | answered and touched nothing | `unsettled`: the lease goes back, and the loop performs again after a rest |
| `idempotent` | may be run again to the same end | the same |
| `uncertain` | may have acted once already: an agent's seat with tools | `unresolved`: nothing is retried by a program |

The class rides on the claim, and the hat says which classes a Work
admits: an answered Work admits all three, and an edit admits no
`effect_free` performer. An outcome names its claim's class. `run` and
`loop` claim with their performer's class; only `next` and `submit`
take `--effect`.

An `uncertain` attempt whose settle failed is marked unresolved at the
head (`attempt.unresolved`, and its effect's result unknown), the leg
files friction naming the attempt, the holder and the lease, and the
loop stops that slot. An `uncertain` claim whose lease lapses with no
outcome is treated the same, which is why such a claim is taken for an
hour by default. A person decides:

```sh
hale dna effect resolve attempt:<attempt> --outcome ok|failed
```

## The worker loop

`hale dna work loop --as position:agent --parallel N` starts `N`
children, each a `run` as its own holder, `position:agent#1` to
`position:agent#N`, so two workers never share a lease. A child that
performed starts again at once; one that found nothing, left the Work
to a person, or gave it back waits its slot's backoff first
(`--idle-ms`, doubling per idle run up to a minute). Each child's
answer is one JSON line as it ends, and the loop's own line comes last.

SIGTERM or SIGINT drains the loop: no child starts again, and each is
told to end and, after three seconds, made to. From another terminal,
`hale dna work loop --drain` writes a marker (`.hale/dna/legs/drain`)
that tells the running loop to take no new work and end once its
children have. A child ended mid-task leaves a lease that expires, and
the owner asks again.

Workers move the record under each other. A head that read it while it
moved answers `snapshot_changed` or `command_busy`; neither admits the
command, and the leg asks again under the same request id for up to
ten seconds.

## The hat

The hat is what a leg reads: one Work's context as structure, never a
prompt. `brief` reads it from the head (`…/dna/context?id=<work>`). It
carries the position and its charter; the practices ratified for the
Work's target, with their ids; the objective, target, output contract,
data class and requirement; the effect classes the Work admits, its
cost ceiling and knowledge bindings; its **hands**, the tools the
output contract gives (each with its description and the schema of its
arguments), the tool grant that names them, and which of them validate
the outcome; the model its mapping chose (mode, size, category,
model and the rule that chose it); the history of the execution the
Work belongs to; and the record head and memory
watermark it was read at, with its digest.

On a head that serves OIDC the hat is the lease holder's: the person your
token maps to, who claimed the attempt and still holds the lease, reads it
exactly as on a local head. Anyone else, and you before `next` claims, is
refused `lease_required`, naming the Work. The claim row is the record's own
answer to who may see this Work's hat; nothing outside the record has to
vouch for it.

The hat reads no clock, no environment value and no random id, so read
twice at one head it is one digest. Rendering is the leg's: `text` for
a person, `prompt` for a model, `agent` for the prompt with each hand
named and the validators to run before handing the outcome back. The renderer's version (`legs-render/2`), the hat's
digest and the digest of the rendering go back with the outcome, so a
replay renders from the recorded hat.

## Hands

What a Work's hands are is its output contract's to say, and the hat
carries them (`dna/core/contracts.hl`, `dna/core/tools.hl`):

| contract | hands | validated by |
| --- | --- | --- |
| `Patch` | `read`, `edit`, `check`, `test`, `fmt`, `patch` | `check`, `test`, `fmt` |
| `Assessment` | `record_status`, `record_history`, `org_chart` (read-only) | — |
| any other | none yet | — |

Every hand runs where the performer is, never in the organism. A
validator is run before the outcome is handed back, and the owner runs
the same check again on submit: only the owner's verdict counts.

A performer is handed `legs::Hands`, a set of interfaces:

| hand | what it does |
| --- | --- |
| git | a scratch clone (never the primary checkout), a patch applied, a commit, the commit's patch |
| forge | a review opened through `gh`, a verdict read; the forge decides, and a leg never merges |
| toolchain | `hale check`, `hale test`, `hale fmt --check` |
| deploy, heart | refuse: neither is a leg's hand yet |
| lease | renews the lease while the performer waits, and asks for its allowance |

A Work whose output contract is `Patch` asks for a change. The leg
commits in its scratch clone and hands back `result_ref: commit:<sha>`
with the commit's patch as a receipt; the owner applies the patch at
the genome's head, verifies it, and opens the Review with the leg as
its author ([Reviews](./head.md#reviews)).

## Voice: the model catalog

The organization calls models through a router over backends that
share one interface, `ModelBackend`. Which model each position calls
is a catalog in source, `dna/org/models.hl`, written by `init` for
what the machine had, and yours to edit. A backend is a constructor
function; a position's router is composed from them:

```hale,fragment
fn frontier() -> dna::OpenAiChat {
    return dna::OpenAiChat { name: "deep", model: "gpt-4o", endpoint: "https://api.openai.com/v1/chat/completions", credential: dna::HostedCredential { key: "OPENAI_API_KEY", scheme: "bearer" }, input_micros_per_1k: 2500, output_micros_per_1k: 10000 };
}
// …
fn agent_models() -> dna::ModelRouter {
    return dna::ModelRouter { quick: harness(), deep: harness(), private: desk() };
}
```

`leader_models()` and `editor_models()` are the Leader's and the
editor's; `agent_models()` is the model leg's. The catalog's
`org_budget()` is the one allowance, which the spine owns
([The spine](./spine.md)).

| backend | speaks | notes |
| --- | --- | --- |
| `dna::OpenAiChat` | the OpenAI chat shape: OpenAI, OpenRouter, vLLM, Ollama | key presented as `scheme: "bearer"` |
| `dna::AnthropicMessages` | Anthropic's native Messages API | key presented as `scheme: "x-api-key"` |
| `dna::LocalModel` | the OpenAI shape, to this machine | no key, no `external_model` effect, any data class |
| `dna::HarnessModel` | an installed harness (`claude`, or `codex`) with its own tools | works in an export of the worktree, under a confinement (`dna::Bubblewrap`) |
| `dna::RecordedModel` | a tape over any of the above | below |
| `dna::FakeModel` | scripted answers | no key; for rehearsal and fixtures |

A hosted backend's call carries the `external_model` effect class, so
the law can keep customer data away from it. `init` writes
`claude-opus-5` and `claude-haiku-4-5-20251001` over
`AnthropicMessages` when the vault holds an Anthropic key, else
`gpt-4o` and `gpt-4o-mini` over `OpenAiChat`, naming `OPENAI_API_KEY`
until you set it; `ollama` on `PATH` gives the desk model, and `claude`
(else `codex`) on `PATH` becomes `harness()`. Its `models` lines say
what each position was given.

**Keys are vault slots.** A hosted backend names its key, never the
bytes: `dna::HostedCredential { key: "OPENAI_API_KEY", scheme: "bearer" }`
reads the vault slot `model-OPENAI_API_KEY`, which you fill from stdin:

```sh
hale dna secret set OPENAI_API_KEY
```

`secret set` takes a name the catalog's `HostedCredential` names (or
`FORGE_TOKEN`, or `OIDC_CLIENT_SECRET`). **No environment variable is
read**, first or otherwise: the credential reads its vault slot and
nothing else, fresh on every call, in the one statement that sends the
request, and never returns the bytes. `init`'s discovery reads the
same slots. A backend whose slot is empty is not permitted, and the
router refuses before the wire. The vault is the one where the call is
made: the local directory (`HALE_VAULT_DIR`, else
`$XDG_CACHE_HOME/hale/vault`, else `~/.cache/hale/vault`), or the
vault at `HALE_VAULT_ADDR` when set ([The skin](./skin.md)).

**The probe.** `hale dna models` builds the catalog beside a one-line
main in `.hale/dna/probe` and sends one small request to each permitted
backend; it starts nothing and journals nothing. On a fresh project
with no key:

```text
$ hale dna models
catalog dna/org/models.hl
backend     slot      model                         answer
frontier    deep      gpt-4o                        not permitted (no credential present)
fast        quick     gpt-4o-mini                   not permitted (no credential present)
…
```

A backend that answers prints `ok`, the time, the cost and the reply's
first line; one that fails prints `refused: …`.

**The model leg's calls.** Before its first call the model performer
asks the spine for the attempt's spend (`allowance`); refused, it makes
no call and hands back `declined`. The model is sent the rendered
prompt alone. A rate-limited call the backend never acted on is tried
again up to three times, a second first and each wait double the last,
the lease renewed before each wait and each wait a row of evidence.
Every call goes back with the outcome as evidence (the adapter that
answered, backend, model, tokens, cost, wall time); the owner journals
it as `model.called` on the attempt, and `hale dna history <task>`
sums it (`usage of <task>: …`).

**The tool loop.** A Work whose hat carries hands, and whose model
mapping makes it a `tools` or `do` Work, is performed as a loop: the
model is offered the hands as tools in the chat API's shape, and every
tool call it asks for is run by the leg, never by the model or the
gateway, its result going back as the next turn, until the model
answers. A `do` Work runs in a scratch clone of the leg's repository at
its `HEAD` (`ModelPerformer { repo }` names another): `read`, `edit`
and `patch` work there, and `check`, `test` and `fmt` run the
toolchain on a path of it. Its answer stands only once the hat's
validators pass on every touched directory that holds Hale source (a
nested seed is its own); a refusal goes back to the model as one more
turn. A `Patch` is then
handed back as one commit on the base, its patch first among the
receipts, which is what the owner applies, verifies again and reviews.
A `tools` Work's hands are the head's reads (`record_status`,
`record_history`, the whole execution an id belongs to, and
`org_chart`), and its answer is text. A hand the hat does not give, or
a path outside the worktree, is refused and the refusal is the call's
answer: `..` or `.git` in any spelling, a control character, or a
symbolic link anywhere on the way, since a link in a clone may lead
anywhere. Every model call is evidence (each turn
its own `Idempotency-Key`, `<attempt>:call:<turn>`), every tool call a
row of the `tool-calls` receipt (the hand, digests of what it was given
and what it answered, why it failed), and the loop ends at
`max_turns` (16) or when the attempt's allowance is spent.

A scripted model drives the loop without a wire: `dna::FakeModel {
turns_dir }` answers turn *n* of a request that offers tools from
`<dir>/<n>.json`, an object with `content` (the answer) and/or
`tool_calls` (each `{name, arguments}`).

**The model mapping.** Every call can go through one OpenAI-compatible
gateway, and which of its models a Work gets is decided by two tables of
rows in the record, each written in the setter's name:

```sh
hale dna models category do.deep coder-l --tools --price-in 300 --price-out 1200
hale dna models category think.quick thinker-s
hale dna models rule default quick
hale dna models rule position:api/dev deep --permit standard
hale dna task create --size standard "…"      # an override for this ask
hale dna models map                           # both tables, as they stand
```

A category is `<mode>.<size>`. The **mode** is the harness's, read off
the Work's hands: `think` with none, `tools` with hands but no
workspace, `do` with a workspace; a `tools` or `do` category needs a
model that returns tool calls (`--tools`). The **size** is the
model's, `quick`, `standard` or `deep`, and the most specific rule
decides it: `position:<name>`, then `contract:<Name>`, then `default`,
else `standard`. A task's override (`--size`, or a `task:<id>` rule)
moves it only inside the sizes the rule beneath it permits. The hat
carries the resolution (`model`: mode, size, category, model and the
rule that chose it, or why no model fills the category), and the model
leg names that model in its request, with the call's `metadata` (the
attempt, position and holder, for attribution only) and an
`Idempotency-Key`. A backend sends `metadata` only with
`send_metadata: true`, for a gateway that keeps it; a provider may
refuse it. With no category filled, the catalog's backend chooses as
before. A `tools` or `do` Work whose category names a model that
returns no tool calls is declined before any spend
(`the model mapping refused <model> for this Work`): fill the
category with one that does (`hale dna models category <mode>.<size>
<model> --tools`).

**The tape.** `dna::RecordedModel { dir, mode, inner }` wraps any
backend. In `record` mode it forwards to `inner` and writes the answer
under a key made of the request's identity (role, backend, the model the
request asks for,
prompt and context digests, data class, grant, for a harness the
workspace's starting tree, and for a tool loop's turn the tools offered
and the turns so far), with the tool calls the model asked for and the
patch a harness made beside it, so a taped loop replays turn by turn.
In `replay` mode it answers from the directory, keyless, and applies
that patch; a miss is refused, naming the request and, when an entry
shares its prompt, the fields that differed. `dir_env` and `mode_env`
name environment variables it reads at birth, as the acceptance
fixture's catalog does (`dna/acceptance/trio.fixture/catalog.hl`):

```hale,fragment
fn frontier() -> dna::RecordedModel {
    return dna::RecordedModel { name: "deep", dir: ".hale/dna/tape", dir_env: "HALE_DNA_TAPE_DIR", mode_env: "HALE_DNA_TAPE", inner: dna::AnthropicMessages { name: "deep", model: "claude-sonnet-5", input_micros_per_1k: 3000, output_micros_per_1k: 15000 } };
}
```

A tape proves how the organization handles recorded outcomes; model
quality is only tested by a fresh run.

## An external harness

A harness of your own needs no performer in `work.hl`. It claims with
`next`, reads `brief --render agent`, asks `allowance` before its first
model call and keeps within the grant, does the work, and hands it back
with `submit --evidence-file calls.json`, a JSON array of
`model.called` bodies, each naming its `adapter`. Calls that cost more
than was granted, or anything when nothing was asked, settle the
attempt `failed`, naming the overrun.

## A person's session

A person can work a Work with an agent of their own, a coding assistant
in their editor, say, and still have the organization's hat and hands.
`hale mcp` gives that agent two tools: `hale_dna_work` for the verbs
(`next` to claim, `brief` for the hat) and `hale_dna_hand` to run one of
the Work's hands by name, with its arguments. Every hand runs on the
person's machine: a file hand (`read`, `edit`, `check`, `test`, `fmt`,
`patch`) in a worktree kept for the attempt under the project's
`.hale/dna/work/`, a clone of the repository at its `HEAD`; an
infrastructure hand through the head's reads. A hand the hat does not
give is refused, as it is to a model leg. When the change is ready,
`submit --from-worktree` runs the hat's validators over every touched
directory holding Hale source, refuses the change if one fails, and
otherwise hands back one commit on the base with its patch, which the
owner applies, verifies and reviews as any leg's; the kept worktree
goes with it.

```sh
hale dna work next --as position:api/dev
hale dna work hand --attempt <attempt> --name read --args '{"path": "api/todo.hl"}'
hale dna work hand --attempt <attempt> --name edit --args '{"path": "api/todo.hl", "text": "…"}'
hale dna work submit --as position:api/dev --attempt <attempt> --token <n> --from-worktree --result "…"
```

## How it breaks

- **``the head's socket lists no `Commands::claim` for <person>``**:
  your uid maps to no person, or the person holds no position. Map the
  uid with `dna.unix.member`, and fill a position (`hale dna fill`).
- **`the model performer declares no effect class`** (exit 2): add
  `effect:` to it in `dna/org/work.hl`.
- **Nothing to claim**: no attempt of that kind was relayed to legs, or
  none fits your `--classes` or `--capabilities`; the refusal names what
  stood in the way.
- **`declined: the model mapping refused <model> for this Work`**: a
  `tools` or `do` Work's category names a model that returns no tool
  calls. Fill it with one that does (`hale dna models category
  <mode>.<size> <model> --tools`).
- **`` `<name>` is not a hand of this Work ``** / **`` `<path>` is outside the
  worktree ``**: the hat's grant and the worktree bound every hand; a
  `..` or `.git` component, a control character or a symbolic link on
  the way is refused.
- **`unresolved`**: an `uncertain` performer may have acted. Read the
  attempt's history, then `hale dna effect resolve`.
- **`declined`**: the budget gate admitted no spend
  ([The spine](./spine.md)).
