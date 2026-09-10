# Shaping it

Everything you might change lives in two files you own, and both are
ordinary Hale source that `hale check` validates. There is no
configuration file, and there is nothing to restart into: edit,
check, and the next `hale dna run` picks it up.

## The purpose

`dna/purpose.hl` is one sentence. Make it true:

```hale,fragment
const PURPOSE: String = "chat: keep the rooms the only path to the signer; every change reviewed by a maintainer.";
```

Changing it re-opens the purpose review — the organism asks you to
ratify the new sentence. It costs nothing and it is the first thing
a reviewer sees about what this program is meant to be.

## How much autonomy

In `dna/assembly.hl`, the grant:

```hale,fragment
boundary: dna::AutonomyBoundary {
    child: "chat",
    grant: dna::Grant { child: "chat", classes: "refactor docs", max_magnitude: 4, review: "pre" }
},
review_policy: dna::HumanBeforeApply { },
```

- **`classes`** — the kinds of change the organism may decide about
  on its own terms. A request through `hale dna ask` is an
  `application` change; with the default grant it always
  *escalates* to you. Widen to `"refactor docs application"` and
  the organism assesses it under the grant instead — and still asks
  you, in this phase, but tells you it would have been within
  bounds.
- **`max_magnitude`** — how big a change may be before it escalates
  regardless of class. Small is safe; raise it as the history
  earns it.
- **`review: "pre"`** — you approve before anything is applied.
  This is the default and the right setting for anything you're not
  sure about.

What you'll see in a review as a result — the *disposition*:

| disposition | meaning |
|---|---|
| `review` | within the grant; asking you because review is `pre` |
| `stage` | within the grant, but not enough evidence yet (a first change, no tests, no replay); asking you |
| `escalate` | outside the grant (class or size); asking you |
| `release` | within the grant, enough evidence, and the policy doesn't block this class: applied first, reviewed after |
| `deny` | the candidate doesn't even check; not a candidate |

Some things escalate whatever the grant says: a change that touches
the rules, reaches a new effect, crosses ownership, or is
irreversible and reaches outside the program. No setting cancels
those.

## Post-review, for what you trust

For small, reversible refactors on a program with a history, let
the organism apply first and ask after:

```hale,fragment
review_policy: dna::PostReviewRefactors { },
boundary: dna::AutonomyBoundary {
    child: "chat",
    grant: dna::Grant { child: "chat", classes: "refactor", max_magnitude: 12, review: "post" }
},
```

A refactor that stays inside the grant, touches no rule or effect,
and comes with clean evidence is applied, restarted and observed
straight away; the review arrives afterwards as `post-review:
applied m4 (refactor): …?`, and a `reviewer` can answer it. Reject
it and it is rolled back. Anything that touches rules or effects
still waits for a maintainer, whatever this says.

## Models

Also in `dna/assembly.hl`. The editor has its own router; the
general-purpose agent has another. Three kinds of backend:

```hale,fragment
models: dna::ModelRouter {
    quick: dna::HostedModel { name: "quick", model: "gpt-4o-mini", credential: dna::HostedCredential { env_var: "OPENAI_API_KEY" } },
    deep: dna::HostedModel { name: "deep", model: "gpt-4o", credential: dna::HostedCredential { env_var: "OPENAI_API_KEY" }, input_micros_per_1k: 2500, output_micros_per_1k: 10000 },
    private: dna::LocalModel { name: "private", endpoint: "http://127.0.0.1:11434/v1/chat/completions", model: "llama3" }
}
```

- **Hosted** — any OpenAI-compatible endpoint. The key is named by
  environment variable and read into a sealed box the rest of the
  program cannot read back from. Without the key, the backend simply
  isn't available. Customer-classed data never goes to it.
- **Local** — the same wire to something on your machine (an
  Ollama-style server). No key, any data.
- **Scripted** — `dna::FakeModel { answer_file: "…" }` returns a
  fixed answer. Useful to rehearse the whole loop on a change you
  wrote yourself, with no key and no cost, and how the acceptance
  test runs in CI.

Costs are metered per call and journaled; a daily ceiling on the
router refuses cleanly when spent. `hale dna history m1/a0` shows
what one attempt asked for and what it cost. The prompt itself is
never recorded, only its hash.

## The rules

`dna_constitution.hl` is the part of the law you can read in one
sitting. As generated it promises: nothing is applied except through
the organism's gate after a review; the editor cannot reach git, the
network, a deployment or the organism's memory; credentials stay
sealed. These hold because the compiler checks them against the
assembly you actually wrote — hand the editor a git handle in
`assembly.hl` and `hale check` fails with the path that proves it.

Add rules of your own in the same block; the vocabulary is the one
in [Claims & the law](../claims.md). Don't remove the generated
ones.

## The observation window

`hale dna run --observe 30`. How long a restarted program must stay
up before a change is kept. The default is fifteen seconds; a
program with a slow start deserves more.

## Keeping the toolchain's part current

`hale dna upgrade` rewrites `vendor/dna` for the `hale` you have and
re-pins `hale.lock`. Your three files are untouched. Run it after
upgrading `hale`.
