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
models  found   ANTHROPIC_API_KEY set, ollama on PATH (llama3)
models  frontier = claude-opus-5 · fast = claude-haiku-4-5-20251001 (ANTHROPIC_API_KEY) · desk = llama3 (ollama at 127.0.0.1:11434)
models  leader, editor, agent: deep = frontier, quick = fast, private = desk · budget 25.00 USD a day (`hale dna models` probes them)
```

With `ANTHROPIC_API_KEY` the hosted backends speak to Anthropic's
OpenAI-compatible endpoint with the strongest models; with
`OPENAI_API_KEY` alone, to OpenAI; with neither, the catalog still
names OpenAI with the key's name as a placeholder, the hosted
backends are simply not permitted until it is set, and every review
waits for the Board. `ollama` on `PATH` makes its first listed model
the desk model. Re-running `init` keeps a catalog you have edited;
`hale dna upgrade` writes one for an organization from before the
catalog and tells you what to point at it.

## Three backends

- **`OpenAiChat`** — a prompt leaves the process for an endpoint
  speaking the OpenAI chat shape (OpenAI, OpenRouter, vLLM,
  Anthropic's compatibility endpoint). `complete` carries the
  `external_model` effect class, so a claim can keep customer data
  away from it structurally, and the router already refuses
  `data_class: "customer"` for it. The API key is read from the
  environment into a **sealed** `HostedCredential` that presents it
  on the wire and never returns it; without a credential the model
  is not a permitted backend, and the router refuses before the
  wire.
- **`LocalModel`** — the same wire to a local endpoint: no
  credential, no `external_model`, any data class.
- **`FakeModel`** — scripted. `answer` (or `answer_file`, or an
  `answers_dir` by role) is returned verbatim, optionally for one
  `answer_role` only; other roles get a deterministic digest answer;
  `fail_after: n` refuses every call after the n-th. This is how the
  acceptance scenarios run in CI without a key, and how you can
  rehearse a session before spending money. It leaves the same
  evidence a hosted call does, with `adapter: fake`.

## Probing the catalog

```text
$ hale dna models
catalog dna/org/models.hl
backend     slot      model                         answer
frontier    deep      claude-opus-5                 ok  1.2s  1230 micro-dollars  "ready"
fast        quick     claude-haiku-4-5-20251001     ok  412ms  38 micro-dollars  "Ready."
desk        private   llama3                        refused: http 0 connect connection refused
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
held the material. See [Claims & the law](../claims.md) on `@sealed`.
