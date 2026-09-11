# Models and credentials

The organization calls models through a **router** over backends
that share one interface, so the law can tell them apart by what
they reach. Which model each position calls is a **catalog in
source**, `dna/org/models.hl`, written by `init` from what your
machine had and yours to edit:

```hale,fragment
// a backend is a constructor function
fn frontier() -> dna::OpenAiChat {
    return dna::OpenAiChat { name: "deep", model: "gpt-4o", endpoint: "https://api.openai.com/v1/chat/completions", credential: dna::HostedCredential { env_var: "OPENAI_API_KEY" }, input_micros_per_1k: 2500, output_micros_per_1k: 10000 };
}
fn fast() -> dna::OpenAiChat { … }
fn desk() -> dna::LocalModel {
    return dna::LocalModel { name: "private", endpoint: "http://127.0.0.1:11434/v1/chat/completions", model: "llama3" };
}

// a position's router is composed from them
fn leader_models() -> dna::ModelRouter {
    return dna::ModelRouter { quick: fast(), deep: frontier(), private: desk() };
}
fn editor_models() -> dna::ModelRouter { … }
fn agent_models() -> dna::ModelRouter { … }

// the organization's allowance, one policy
fn org_budget() -> dna::BudgetPolicy {
    return dna::BudgetPolicy { window: "day", allowance_micros: 25000000 };
}
```

The generated organization takes its three routers from here —
`models: leader_models()` on the Leader, `editor_models()` on the
`SourceEditor`, `agent_models()` on the `AgentPerformer` — and names
no adapter itself. They can differ, and the split that makes sense
is the obvious one: the judgement of what gets applied on the
strongest model, the production of candidates on a cheaper one. A
new position as the organization grows is one more router function;
switching provider is this one file. `hale check` validates all of
it.

## What init found

`init` looks at the environment and `PATH` and writes the catalog
for what is there, then says so:

```text
created …/chat/dna/org/models.hl
models  found   ANTHROPIC_API_KEY set, ollama on PATH (llama3), claude on PATH
models  frontier = claude-opus-5 · fast = claude-haiku-4-5-20251001 (ANTHROPIC_API_KEY) · desk = llama3 (ollama at 127.0.0.1:11434)
models  editor, agent: quick = harness (claude), deep = frontier · leader: deep = frontier, quick = fast · private = desk · budget 25.00 USD a day (`hale dna models` probes them)
```

With `ANTHROPIC_API_KEY` the hosted backends are `AnthropicMessages`
with the strongest models; with `OPENAI_API_KEY` alone, `OpenAiChat`
to OpenAI; with neither, the catalog still names OpenAI with the
key's name as a placeholder, the hosted backends are simply not
permitted until it is set, and every review waits for the Board. `ollama` on `PATH` makes its first listed model
the desk model. `claude` (else `codex`) on `PATH` becomes
`harness()`: the editor's and the agent's quick tier, and with no key
at all every model-backed slot, so a laptop with a harness and no key
still edits, reviews and decides. Re-running `init` keeps a catalog you have edited;
`hale dna upgrade` writes one for an organization from before the
catalog and tells you what to point at it.

## Five backends

- **`OpenAiChat`** — a prompt leaves the process for an endpoint
  speaking the OpenAI chat shape (OpenAI, OpenRouter, vLLM, Ollama).
  `complete` carries the `external_model` effect class, so a claim
  can keep customer data away from it structurally, and the router
  already refuses `data_class: "customer"` for it. The API key is
  read from the environment into a **sealed** `HostedCredential`
  that presents it on the wire and never returns it; without a
  credential the model is not a permitted backend, and the router
  refuses before the wire.
- **`AnthropicMessages`** — the native Messages API, on the same
  seam with the same evidence: the context travels as `system`, the
  prompt as the user message, `max_tokens` is always sent (the API
  requires it), the reply's text blocks are joined. Its credential
  has `scheme: "x-api-key"`; the credential, not the adapter, knows
  how the key is presented — `bearer` for everything OpenAI-shaped —
  so no adapter ever composes a header with the material in it.
- **`HarnessModel`** — an installed coding harness (`claude`, or
  `codex` with `output: "text"`), run per request with its own tools
  on. It does not answer with a file: it *works in place*, and the
  editor treats it accordingly (below). Customer-classed data never
  goes to it, and it is not a permitted backend when the binary is
  not on `PATH`.
- **`LocalModel`** — the same wire to a local endpoint: no
  credential, no `external_model`, any data class.
- **`FakeModel`** — scripted. `answer` (or `answer_file`, or an
  `answers_dir` by role) is returned verbatim, optionally for one
  `answer_role` only; other roles get a deterministic digest answer;
  `fail_after: n` refuses every call after the n-th. This is how the
  acceptance scenarios run in CI without a key, and how you can
  rehearse a session before spending money. It leaves the same
  evidence a hosted call does, with `adapter: fake`.

## The harness works in an export

The DNA has no opinion about how a change gets made; its law is
about reach. So a harness runs with all its tools, but in an
**export** of the Mutation's worktree: a plain directory with the
same files and no `.git`, no `.hale`. The editor gives it the whole
objective in one prompt, waits, and then imports the export's diff
back under the grant — a changed or new file is written through the
same tools a chat model's edit goes through, a deleted one is
removed, and anything the harness touched outside the grant is
counted and left behind. `files_changed` comes from that diff, never
from what the harness said. Then the usual fmt, check, a retry with
the diagnostics in the next prompt, and the assessment.

The export is a boundary on what DNA accepts, not a sandbox: a
process whose working directory is the export can still reach
whatever your account can. What DNA guarantees is narrower and
enforced: **the genome is unreachable from the harness process.**
That is a `Confinement`:

```hale,fragment
fn harness() -> dna::HarnessModel {
    return dna::HarnessModel { name: "quick", command: "claude", confinement: dna::Bubblewrap { } };
}
```

`Bubblewrap` (Linux) runs the harness with the filesystem as you see
it — its own settings and login, the toolchain, the network — and
the repository and the worktree replaced by empty mounts, so the
genome does not exist for it. With no confinement available the
harness is refused, unless you say `allow_unconfined: true`; the
evidence records which it was (`confinement=bubblewrap`,
`confinement=none`). And on every platform the organization checks
that neither the repository's head nor the worktree's moved while an
attempt ran, and fails the Mutation by the record if they did. Outside
the genome the harness has exactly your account, which is what
running it by hand has.

Evidence is one `model.called` row per call, with the harness's own
cost summary, `tool_grant: harness @export`, and the confinement.
The Leader can use a harness too: an answer-only call runs in an
empty directory of its own.

## Probing the catalog

```text
$ hale dna models
catalog dna/org/models.hl
backend     slot      model                         answer
frontier    deep      claude-opus-5                 ok  1.2s  1230 micro-dollars  "ready"
fast        quick     claude-haiku-4-5-20251001     ok  412ms  38 micro-dollars  "Ready."
desk        private   llama3                        refused: http 0 connect connection refused
harness     quick     claude                        ok  6.1s  4100 micro-dollars  "ready"
```

One line per backend the catalog names, one small request to each
that is permitted (`not permitted (no credential present)` for a
hosted backend without its key). The organization is not started
and nothing is journaled: a probe is nobody's Attempt. Run it after
editing the catalog, and before the first `hale dna run`.

## The budget

The catalog's `org_budget()` is one policy for the whole
organization: an allowance in micro-dollars per `day`, `week`, or
`none` (one allowance for the record's lifetime); `0` is unmetered.
The substrate owns the one counter. Every model call's evidence is
accounted at its time, the counter is rebuilt from the record when
the organization starts (a restart loses nothing, and a position you
add later is covered without doing anything), and when a window is
spent:

- `hale dna ask` is refused with the reason: `refused: budget
  exhausted (spent 25000000 of 25000000 micro-dollars this day in 41
  call(s))`;
- `budget.exhausted` is journaled once for the window and the
  membrane is told;
- a pending Review is not announced to the Leader; it waits for you
  (`hale dna review <id> approve`), which needs no model.

The next window starts clean. Nothing else in the organization
carries an allowance: naming the budget from several places cannot
create several.

## Selection

The router picks a class per request: the deep tier for `review`
and for `assurance: 2` (the editor's fitness assessment), the private
tier when the data class forbids leaving the process, the quick tier
otherwise; it falls through to whatever is permitted and has the
capacity. Which backend answered is recorded, never hidden; what it
cost is counted here and accounted by the budget.

## Evidence, never the prompt

Every call publishes `ModelCalled` and the substrate journals it as
`model.called` on the attempt id:

```text
   11  model.called   m1/a0   {"adapter": "fake", "backend": "quick", "endpoint": "fake://quick", "credential": "", "requested_model": "fake-1", …, "prompt_digest": "…", "context_digest": "…", "tool_grant": "read edit fmt check @.hale/dna/worktrees/m1", "response_digest": "…", "ok": true, …}
```

Adapter and endpoint, the credential's fingerprint (a correlation
handle, never the material), requested and reported model,
parameters as sent, prompt and context digests, the knowledge
bindings used, the tool grant, the response digest, tokens, wall
time, cost, validation, retry lineage, data class, the refusal
when there was one, and when the call was made (`at`, which is what
the budget counts by). `hale dna history m1/a0` shows one attempt's
calls; the Leader's review calls are on the Review's id.

## The editor's grant

The `SourceEditor`'s hands are `WorktreeTools`: `read` and `edit` of
worktree-relative paths (an absolute path or a `..` segment is
refused before any model is called, and counted), `fmt` and `check`
against the worktree. The substrate points the grant at the
Mutation's worktree; the grant itself cannot widen its root. The
editor plans over files — which ones the objective names — edits
each, and retries a candidate that does not check up to `max_tries`
times with the diagnostics in hand. Every model request it makes
carries the grant as `tool_grant`, so the evidence says what the
Attempt could touch. What the editor holds *not* — repository,
worktree gateway, deployment, Knowledge — is law ([What init
makes](./attach.md)).

## Sealed credentials

`HostedCredential` is `@sealed`: its params are readable only from
inside it, so no holder can read the key back, and the generated law
requires it (`credentials_sealed: require sealed(all credentials)`).
It takes the *name* of a source (`env_var`), never the bytes, and
loads at birth, so there is no construction site at which anything
held the material. Its `scheme` says how the wire wants the key —
`bearer` or `x-api-key` — and the credential composes that header
itself; an adapter hands it only its own headers. See [Claims & the law](../claims.md) on `@sealed`.
