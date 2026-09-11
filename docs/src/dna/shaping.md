# Shaping it

Everything you might change lives in three files you own, and all
three are ordinary Hale source that `hale check` validates. There is
no configuration file, and there is nothing to restart into: edit,
check, commit, and the next `hale dna run` picks it up — or ask the
organization to change them, and review the change like any other.

## The purpose

`dna/org/purpose.hl` is one sentence. Make it true:

```hale,fragment
const PURPOSE: String = "chat: keep the rooms the only path to the signer; every change reviewed.";
```

Changing it re-opens the purpose review — the organization asks the
Board to ratify the new sentence. It costs nothing and it is the
first thing a reviewer sees about what this codebase is meant to be.

## The grant

In `dna/org/main.hl`:

```hale,fragment
boundary: dna::AutonomyBoundary {
    child: "chat",
    grant: dna::Grant { child: "chat", classes: "refactor docs", max_magnitude: 4, review: "pre" }
},
review_policy: dna::OrgPolicy { },
```

- **`classes`** — the kinds of change the Leader may decide. A
  request through `hale dna ask` is an `application` change; with
  the default grant it *escalates* to the Board. Widen to
  `"refactor docs application"` and the Leader decides those too —
  and the record shows it did.
- **`max_magnitude`** — how big a change may be before it escalates
  regardless of class. Small is safe; raise it as the record earns
  it.
- **`review: "pre"`** — a verdict before anything is applied. The
  default, and the right setting for anything you're not sure about.

What you'll see in a review as a result — the *disposition*:

| disposition | meaning |
|---|---|
| `review` | within the grant; the Leader is asked because review is `pre` |
| `stage` | within the grant, but not enough evidence yet (a first change, no tests); still asked |
| `escalate` | outside the grant (class or size); the Board is asked |
| `release` | within the grant, enough evidence, and the policy doesn't block this class: applied first, reviewed after |
| `deny` | the candidate doesn't check, or breaks the fleet; not a candidate |

Some things go to the Board whatever the grant says: a change that
touches the law, reaches a new effect class, crosses ownership, or
changes the fleet's shape. No setting cancels those.

## Post-review, for what you trust

For small, reversible refactors on a codebase with a history, let
the organization apply first and ask after:

```hale,fragment
review_policy: dna::PostReviewRefactors { },
boundary: dna::AutonomyBoundary {
    child: "chat",
    grant: dna::Grant { child: "chat", classes: "refactor", max_magnitude: 12, review: "post" }
},
```

A refactor that stays inside the grant, touches no rule or effect,
and comes with clean evidence is applied, expressed and observed
straight away; the review arrives afterwards as `post-review:
applied m4 (refactor): …?`, and a `reviewer` can answer it. Reject it
and it is rolled back.

## Models

In `dna/org/models.hl`, the catalog: a backend is a constructor
function, each position's router is composed from them, and the
budget is one policy. `init` wrote it from what your machine had
(the keys in the environment, `ollama` on `PATH`) and said what each
position was given; `hale dna models` probes every backend it names.

```hale,fragment
fn frontier() -> dna::OpenAiChat { return dna::OpenAiChat { name: "deep", model: "gpt-4o", endpoint: "https://api.openai.com/v1/chat/completions", credential: dna::HostedCredential { env_var: "OPENAI_API_KEY" }, input_micros_per_1k: 2500, output_micros_per_1k: 10000 }; }
fn fast() -> dna::OpenAiChat { return dna::OpenAiChat { name: "quick", model: "gpt-4o-mini", endpoint: "https://api.openai.com/v1/chat/completions", credential: dna::HostedCredential { env_var: "OPENAI_API_KEY" }, input_micros_per_1k: 150, output_micros_per_1k: 600 }; }
fn desk() -> dna::LocalModel { return dna::LocalModel { name: "private", endpoint: "http://127.0.0.1:11434/v1/chat/completions", model: "llama3" }; }

fn leader_models() -> dna::ModelRouter { return dna::ModelRouter { quick: fast(), deep: frontier(), private: desk() }; }
fn editor_models() -> dna::ModelRouter { return dna::ModelRouter { quick: fast(), deep: frontier(), private: desk() }; }
fn agent_models() -> dna::ModelRouter { return dna::ModelRouter { quick: fast(), deep: frontier(), private: desk() }; }

fn org_budget() -> dna::BudgetPolicy { return dna::BudgetPolicy { window: "day", allowance_micros: 25000000 }; }
```

- **Hosted** (`OpenAiChat` for any endpoint speaking the OpenAI chat
  shape, `AnthropicMessages` for Anthropic's native API) — the key
  is named by environment variable and read into a sealed box the
  rest of the program cannot read back from, which presents it the
  way its `scheme` says (`bearer`, or `x-api-key`). Without the key,
  the backend simply isn't available. Customer-classed data never
  goes to it.
- **Local** — the same wire to something on your machine. No key,
  any data.
- **Scripted** — `dna::FakeModel { answer_file: "…" }` returns a
  fixed answer. Useful to rehearse the whole loop on a change you
  wrote yourself, with no key and no cost, and how the acceptance
  tests run in CI.

The Leader reviews with the deep tier. A frontier model there and a
cheaper one on the editor is a reasonable split: the expensive
judgement on what gets applied, the cheap work on producing it. A
new position gets its own router function. Costs are metered per
call, journaled, and counted against the one budget; when a window
is spent, intent is refused with the reason and the Reviews wait for
you. `hale dna history m1/a0` shows what one attempt asked for and
what it cost. The prompt itself is never recorded, only its hash.

## The backends

The organization names what fills each role, and `hale check` sees
the wiring:

| role | fills it |
|---|---|
| the record | `GitJournal` — `refs/dna/journal`; always |
| the membrane (where people decide) | the sockets on one machine; the record across clones; GitHub with `git config dna.github` |
| the deployment | none (`hale dna dev` expresses here); the fleet (`[dna] fleet`); `ShellDeployment { command, seed }` for your own pipeline |
| the observation | the window the host watches; what the nodes report; your command's exit code |

[Operating the fleet](./operating.md) has the fleet and the shell
gateway; [Working with it](./working.md) has GitHub.

## The law

`dna/org/law.hl` is the part of the law you can read in one sitting.
As generated it promises: nothing is applied except through the
substrate's gate after a settled Review; the editor cannot reach
git, the worktree gateway, the deployment or the organization's
memory; the Leader decides and never commits; credentials stay
sealed. These hold because the compiler checks them against the
organization you actually wrote. Add rules in the same block; the
vocabulary is [Claims & the law](../claims.md). Don't remove the
generated ones.

## The observation window

`hale dna dev --observe 30`, `hale dna run --observe 30`. How long a
restarted program — or every touched instance of the fleet — must
stay up before a change is kept. The default is fifteen seconds; a
program with a slow start deserves more.

## Keeping the toolchain's part current

`hale dna upgrade` rewrites `vendor/dna` for the `hale` you have and
re-pins `hale.lock`. Your three files are untouched. Run it after
upgrading `hale`.
