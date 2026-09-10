# Models and credentials

The organization calls models through a **router** over backends
that share one interface, so the law can tell them apart by what
they reach:

```hale,fragment
models: dna::ModelRouter {
    quick: dna::HostedModel { name: "quick", model: "gpt-4o-mini", credential: dna::HostedCredential { env_var: "OPENAI_API_KEY" } },
    deep: dna::HostedModel { name: "deep", model: "gpt-4o", credential: dna::HostedCredential { env_var: "OPENAI_API_KEY" }, input_micros_per_1k: 2500, output_micros_per_1k: 10000 },
    private: dna::LocalModel { name: "private", endpoint: "http://127.0.0.1:11434/v1/chat/completions", model: "llama3" }
}
```

The generated organization wires three routers: one for the
`AgentPerformer` (general Work), one for the `SourceEditor`, one for
the `Leader`. They can differ, and the split that makes sense is the
obvious one: the judgement of what gets applied on the strongest
model, the production of candidates on a cheaper one.

## Three backends

- **`HostedModel`** — a prompt leaves the process for an
  OpenAI-compatible chat endpoint. `complete` carries the
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
  `answer_role` only; other roles get a deterministic digest answer.
  This is how the acceptance scenarios run in CI without a key, and
  how you can rehearse a session before spending money. It leaves
  the same evidence a hosted call does, with `adapter: fake`.

## Selection

The router picks a class per request: the deep tier for `review`
and for `assurance: 2` (the editor's fitness assessment), the private
tier when the data class forbids leaving the process, the quick tier
otherwise; it falls through to whatever is permitted and has the
capacity. Which backend answered is recorded, never hidden. A daily
cost ceiling refuses cleanly when spent.

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
time, cost, validation, retry lineage, data class, and the refusal
when there was one. `hale dna history m1/a0` shows one attempt's
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
