# Attaching it

`hale dna init` attaches the DNA to an application that already has
a `main locus`; `hale dna new <name>` makes a greenfield one with the
DNA already attached. Both are idempotent, and `init` rewrites
nothing you wrote — it inserts.

```sh
hale dna new demo            # a greenfield application with its DNA
hale dna init .              # attach to an existing application
hale dna upgrade             # re-materialize vendor/dna for this toolchain
```

Here is `init` on a small chat server (the one in
`dna/acceptance/chat-server`):

```text
$ hale dna init .
ok: 1 file(s) typechecked
wrote   vendor/dna (13 file(s) written, 0 unchanged; hale.lock pins toolchain 0.19.2)
cut     .hale/dna/baseline.topology (schema 1.19, shape 3853f0f14bbf1639, verdict clean)
created dna/purpose.hl
created dna/assembly.hl
created dna_constitution.hl
edited  main.hl (imports, `genome` param, `adopt Project`, membrane bindings)
edited  hale.toml ([claims] base, [environments.local])
seeded  .hale/dna/journal.jsonl (17 event(s): application.attached, structure.observed, responsibility.proposed, review.requested)
edited  .gitignore (/vendor/, /.hale/)
```

`init` refuses an application that does not pass `hale check`, and
it cuts the application's topology artifact *before* it changes
anything, so the baseline is the application as you had it.

## What is generated, and who owns it

| path | owner | what |
|---|---|---|
| `vendor/dna/*.hl` | toolchain | the DNA core, pinned in `hale.lock` as `[dna] toolchain`; git-ignored, re-materialized by `upgrade` |
| `dna/assembly.hl` | project | the **Genome**: the `Dna` constructor with local defaults, and the baseline Review |
| `dna_constitution.hl` (in the app's seed) | project | the groups and `constitution Project` |
| `dna/purpose.hl` | project | the declared purpose the first Review ratifies |
| `.hale/dna/journal.jsonl` | the organism | the Journal, seeded from the compiler's model |
| `.hale/dna/baseline.topology` | the organism | the artifact it was seeded from |

`.hale/` is git-ignored as a whole: the Journal, the membrane
sockets, the worktrees, the evidence and the status projection all
live there and none of them belong in the genome.

## What changes in your main

The diff to the chat server's `main.hl` is the whole of it:

```diff
+import "vendor/dna" as dna;
+import "dna" as genome;
 …
 main locus ChatServer {
     params {
         …
+        genome: genome::Genome = genome::Genome { };
     }
     claims {
         adopt Chat;
+        adopt Project;
     }
     …
+    bindings {
+        dna::ReviewVerdict: unix(".hale/dna/hale-dna.review.verdict.sock", role: listen);
+        dna::IntentOffered: unix(".hale/dna/hale-dna.intent.offered.sock", role: listen);
+        dna::ExpressionObserved: unix(".hale/dna/hale-dna.expression.observed.sock", role: listen);
+    }
 }
```

The `genome` param is the organism: the whole DNA is an ordinary
child of your main locus, born with it, observed with it. The three
bindings are the **membrane** — the typed topics on which a human's
verdict, a human's intent, and the host's observation report enter.
Nothing decides in the transport; the loci that own those topics
decide.

`hale.toml` gains `[claims] base = "Project"` and a `local`
environment, so `hale check --matrix .` judges the entrypoint
against the law:

```text
$ hale check --matrix .
=== ./. @ local ===
ok: 2 file(s) typechecked

ok: 1 (entrypoint, environment) pair(s) checked
```

And the application's own tests still pass — the first of the
twelve acceptance steps is exactly that: an existing application
adds the DNA through ordinary project machinery and loses nothing.

## The Genome

There is no configuration file. Every "setting" is a constructor
argument in `dna/assembly.hl`, and `hale check` sees all of it:

```hale,fragment
locus Genome {
    params {
        core: dna::Dna = dna::Dna {
            journal: dna::FileJournal { path: ".hale/dna/journal.jsonl" },
            work: dna::WorkSystem { agent: dna::AgentPerformer { models: dna::ModelRouter { … } } },
            boundary: dna::AutonomyBoundary {
                child: "chat",
                grant: dna::Grant { child: "chat", classes: "refactor docs", max_magnitude: 4, review: "pre" }
            },
            review_policy: dna::HumanBeforeApply { },
            membrane: dna::LocalHumanMembrane { who: "operator" },
            gateway: dna::MutationGateway {
                workspaces: dna::IsolatedWorktrees { repo: ".", root: ".hale/dna/worktrees" },
                repo: dna::LocalGit { repo: "." }
            },
            verification: dna::HaleVerification { evidence_dir: ".hale/dna/evidence", repo: ".", seed: "." },
            editor: dna::SourceEditor { models: dna::ModelRouter { … } },
            genome_seed: "."
        };
        purpose: dna::Review = dna::Review {
            review_id: "purpose",
            question: "ratify the declared purpose?",
            subject_digest: "sha256:…",
            required_authority: "maintainer",
            author: "hale dna init"
        };
    }
}
```

That is the point of making it source: the law in
`dna_constitution.hl` is evaluated over the assembly you actually
constructed, not over a policy written in prose. Swap
`HumanBeforeApply` for `PostReviewRefactors`, or hand the editor a
`LocalGit`, and the reviewer of that commit sees a law change or a
build failure, not a settings diff.

## The law

`constitution Project` lives in the application's own seed, because
a constitution names groups its adopting entrypoint must declare
(`organism`), and whoever imports the application — its tests — must
see the law and its vocabulary together. As generated:

```hale,fragment
group organism = { ChatServer };
group genome = { genome::Genome };
group dna_gate = { dna::Dna };
group performers = { dna::AgentPerformer, dna::HumanWorkGateway, dna::ServicePerformer, dna::ScriptedPerformer, dna::SourceEditor };
group credentials = { dna::CredentialSource, dna::HostedCredential };
group editors = { dna::SourceEditor, dna::WorktreeTools };
group knowledge = { dna::Knowledge };

constitution Project {
    apply_gated: forbid reaches(genome, effects(genome_apply)) avoiding dna_gate;
    performers_never_apply: forbid reaches(performers, effects(genome_apply));
    credentials_sealed: require sealed(all credentials);
    editors_never_commit: forbid reaches(editors, effects(repo_write));
    editors_never_touch_worktrees: forbid reaches(editors, effects(worktree_io));
    editors_never_apply: forbid reaches(editors, effects(genome_apply));
    editors_never_learn: forbid reaches(editors, knowledge);
    organism_gated: forbid reaches(organism, effects(genome_apply)) avoiding dna_gate;
}
```

Read it as four promises. A mutation is applied only *through* the
assembly's gate, never by a performer. Credentials stay sealed. The
Attempt that edits source holds nothing but its worktree grant — no
git, no worktree gateway, no apply, no Knowledge — and a wiring that
hands it any of them fails `hale check` with a witness path. And
the application itself never reaches an apply except through the
gate.

That last clause, `organism_gated`, is generated **active only when
the baseline artifact has no unresolvable edges**: a `forbid
reaches` over an application with an indirect call fails closed,
and a law that fails the application on day one is not a law anyone
keeps. Otherwise it is written out commented with the reason and the
deferral is journaled as `law.deferred`. Resolve the edges and
uncomment it.

## The purpose, and the first Review

`dna/purpose.hl` is one string, and the Genome's first Review asks a
maintainer to ratify it:

```text
$ hale dna status
organism:   not running — reading the Journal
journal:    17 event(s), chain verified
expression: attached ChatServer (shape 3853f0f14bbf1639) · current shape not cut · build not built
intents:    0 offered, 0 refused
tasks:      none
reviews:    1 pending of 1
  purpose [pending] needs maintainer — ratify the declared purpose?
mutations:  0 (none applies before a human's verdict on the exact candidate)
```

The Review's subject digest is the sha256 of the purpose text.
Change the text and the digest together, or the verdict is refused
as stale — which is the same rule every later Review applies to a
candidate commit.

## The seeded Journal

`init` writes one event per fact the compiler can vouch for, with
provenance, so the organism starts with a memory it did not make up:

- `application.attached` — the entrypoint, the artifact's digests, the toolchain;
- `structure.observed` — one per locus (params, methods, publishes, subscribes, supervision, instances), topic, binding, effect class and claim, provenance `observed`;
- `responsibility.proposed` — one guess per locus from what it reacts to and emits, provenance `inferred`, `ratified: false`, never anything stronger;
- `law.deferred` — a clause the application cannot certify yet;
- `review.requested` — the purpose Review.

The chain is the one the core's `FileJournal` writes (each row's
digest covers the previous), so the organism rehydrates it at birth
and continues it. `hale dna history` prints it:

```text
$ hale dna history
journal .hale/dna/journal.jsonl — 17 event(s), chain verified
    0  application.attached   .                            {"artifact":".hale/dna/baseline.topology",…
    1  structure.observed     locus:ChatServer             {"instances":[{"domain":"main","owner":null,"path":"ChatServer"}],…
    2  responsibility.proposed locus:ChatServer             {"provenance":"inferred","ratified":false,"responsibility":"the entrypoint and root…
    3  structure.observed     locus:Doorman                …
   15  structure.observed     claim:guests_sign_only_via_rooms {"form":"forbid reaches(participants, effects(secret_use)) avoiding rooms",…
   16  review.requested       review:purpose               {"author":"hale dna init",…,"question":"ratify the declared purpose?",…
```
