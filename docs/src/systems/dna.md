# DNA: a governed application

**DNA** is the part of a Hale application that decides how the
application itself changes: what may be attempted, who reviews it,
what evidence counts, and what is never applied without a human.
It is ordinary Hale source. The core (`dna/core` in the hale
repository) ships inside the toolchain; a project materializes it
and owns its own assembly on top.

```sh
hale dna new demo          # a greenfield application with its DNA
hale dna init .            # attach the DNA to an existing application
hale dna upgrade           # re-materialize vendor/dna for this toolchain
```

## What `init` produces

| path | owner | what |
|---|---|---|
| `vendor/dna/*.hl` | toolchain | the DNA core, pinned in `hale.lock` as `[dna] toolchain` |
| `dna/assembly.hl` | project | the `Genome`: the `Dna` constructor with local defaults, and the baseline `Review` |
| `dna_constitution.hl` (in the app's seed) | project | the groups and `constitution Project` |
| `dna/purpose.hl` | project | the declared purpose the first Review ratifies |
| `.hale/dna/journal.jsonl` | the organism | the Journal, seeded from the compiler's model |
| `.hale/dna/baseline.topology` | the organism | the artifact it was seeded from |

The application's `main locus` gains the imports, a `genome` param,
`adopt Project;` and the two membrane bindings;
`hale.toml` gains `[claims] base = "Project"` and a `local`
environment so `hale check --matrix` judges the entrypoint against
the law. Nothing existing is rewritten, and re-running keeps every
file.

There is no configuration file. Every "setting" is a constructor
argument in `dna/assembly.hl`:

```hale,fragment
core: dna::Dna = dna::Dna {
    journal: dna::FileJournal { path: ".hale/dna/journal.jsonl" },
    boundary: dna::AutonomyBoundary { child: "demo", grant: dna::Grant { classes: "refactor docs", max_magnitude: 4, review: "pre" } },
    review_policy: dna::HumanBeforeApply { },
    deployment: dna::NoDeployment { },
    membrane: dna::LocalHumanMembrane { who: "operator" }
};
```

`hale check` sees all of it, which is the point: the law in
`dna/constitution.hl` is evaluated over the assembly you actually
constructed, not over a policy you wrote in prose.

## The seeded Journal

`init` cuts the application's topology artifact and writes one
event per fact the compiler can vouch for, with provenance:

- `application.attached` — the entrypoint, the artifact's digests, the toolchain;
- `structure.observed` — one per locus (params, methods, publishes, subscribes, supervision, instances), topic, binding, effect class and claim — provenance `observed`;
- `responsibility.proposed` — one guess per locus, from what it reacts to and emits — provenance `inferred`, `ratified: false`, never anything stronger;
- `law.deferred` — a clause the application cannot certify yet (see below);
- `review.requested` — the baseline review: ratify `dna/purpose.hl`, pinned to the sha256 of its text.

The chain is the same the core's `FileJournal` writes (each row's
digest covers the previous digest), so the organism rehydrates it
at birth and continues it.

## The law an application can carry

`constitution Project` lives in the application's own seed, because
a constitution names groups its adopting entrypoint must declare
(`organism`) and whoever imports the application (its tests) must
see the law and its vocabulary together. It starts from the core's
own law: a mutation
is applied only through the assembly's gate, performers never
apply, credentials stay sealed. The assembly-scoped clause
(`apply_gated`, quantified from the `Genome`) always holds. The
application-wide clause (`organism_gated`) is generated **active
only when the baseline artifact has no unresolvable edges**: a
`forbid reaches` over an application with an indirect call fails
closed, and a law that fails the application on day one is not a
law anyone keeps. Otherwise it is written out commented with the
reason, and the deferral is journaled. Resolve the edges and
uncomment it.

In Phase 1 the deployment gateway is `NoDeployment`: every
mutation stops at `staged`. That is not a claim, it is the
constructor.

## Running it

```sh
hale dna run            # in the project; --port N for iris, --no-iris to skip it
```

`run` is a **stateless host**. It cuts a fresh artifact of what is
about to run (`.hale/dna/current.topology`), builds, execs the
organism under `LOTUS_OBS=1` from the project root, waits for the
membrane sockets, and attaches iris with the law view on the fresh
artifact, the review view diffing it against the baseline `init`
cut (which is the application as `init` found it, so the first run's
review shows exactly what the DNA added), and the membrane panel. It holds no Task state: an intent
offered through the membrane lands in the organism's Journal, and
when the organism exits the host reaps iris and exits with the
organism's code. The Journal is the record either way.

## The membrane

The organism binds two typed topics on unix sockets under
`.hale/dna/`: `dna.review.verdict` and `dna.intent.offered`. Iris
(`hale iris --membrane .hale/dna`) and `hale dna ask` publish on
them; the Review checks the reviewer's authority, and intent goes
through the membrane gate. See [Iris](./iris.md).
