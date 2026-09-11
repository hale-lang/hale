# What init makes

[Getting started](./getting-started.md) is what you see. This is
what `init` does, and why the pieces sit where they sit.

`init` typechecks the application, materializes the core into
`vendor/dna` (pinned in `hale.lock` as `[dna] toolchain`), cuts the
application's topology artifact **before** anything else (the
baseline), generates the organization under `dna/org`, adds two
environments and `[claims] no_base = true` to `hale.toml`, seeds the
record from the artifact, and formats what it wrote. The application
is not modified: no import, no param, no binding. Re-running keeps
every file.

## The organization is a program

`dna/org/main.hl` is one `main locus` with four children:

```hale,fragment
main locus Org {
    params {
        core: dna::Dna = dna::Dna {
            journal: dna::GitJournal { repo: "." },
            work: dna::WorkSystem { agent: dna::AgentPerformer { name: "agent", models: … } },
            boundary: dna::AutonomyBoundary { child: "chat", grant: dna::Grant { … } },
            review_policy: dna::OrgPolicy { },
            membrane: dna::Board { who: "board" },
            gateway: dna::MutationGateway { leases: dna::GitLeases { repo: "." }, workspaces: dna::IsolatedWorktrees { … }, repo: dna::LocalGit { repo: "." } },
            verification: dna::HaleVerification { receipts: dna::GitReceipts { repo: "." }, scratch: ".hale/dna/scratch", repo: ".", seed: "." },
            editor: dna::SourceEditor { name: "editor", models: … },
            genome_seed: "."
        };
        leader: dna::Leader = dna::Leader { name: "leader", models: …, receipts: dna::GitReceipts { repo: "." }, source: dna::SourceReader { repo: "." } };
        purpose: dna::Review = dna::Review { review_id: "purpose", question: "ratify the declared purpose?", subject_digest: "sha256:…", required_authority: "board", author: "hale dna init" };
    }
    claims { adopt Org; }
    bindings {
        dna::ReviewVerdict: unix(".hale/dna/hale-dna.review.verdict.sock", role: listen);
        dna::IntentOffered: unix(".hale/dna/hale-dna.intent.offered.sock", role: listen);
        dna::ExpressionObserved: unix(".hale/dna/hale-dna.expression.observed.sock", role: listen);
        dna::PressureRaised: unix(".hale/dna/hale-dna.pressure.raised.sock", role: listen);
    }
    run() { while true { std::time::sleep(100ms); } }
}
```

`core` is the substrate: the record, the work system, the grant and
the policy, the Board as the membrane, the gateway, verification,
the editor. `leader` is the position that decides inside the grant.
`purpose` is the first Review. The four bindings are the
**membrane** — the typed topics on which a verdict, an intent, the
host's observation report and a pressure signal enter. Nothing
decides in the transport; the loci that own those topics decide.

Every setting is a constructor argument, so `hale check` sees the
whole organization as wiring: which position holds which handle,
which effects each can reach. That is what the law is checked
against.

## The law

`dna/org/law.hl` names groups over the core's loci and states what
may never reach what:

```hale,fragment
group board = { dna::Board };
group leader = { dna::Leader };
group substrate = { dna::Dna };
group positions = { dna::Leader, dna::SourceEditor, dna::WorktreeTools, dna::AgentPerformer, … };
group editors = { dna::SourceEditor, dna::WorktreeTools };
group knowledge = { dna::Knowledge };
group credentials = { dna::CredentialSource, dna::HostedCredential };

constitution Org {
    apply_only_through_the_substrate: forbid reaches(positions, effects(genome_apply)) avoiding substrate;
    editors_never_commit: forbid reaches(editors, effects(repo_write));
    editors_never_touch_worktrees: forbid reaches(editors, effects(worktree_io));
    editors_never_apply: forbid reaches(editors, effects(genome_apply));
    editors_never_learn: forbid reaches(editors, knowledge);
    leader_never_commits: forbid reaches(leader, effects(repo_write)) avoiding substrate;
    leader_never_touches_worktrees: forbid reaches(leader, effects(worktree_io)) avoiding substrate;
    credentials_sealed: require sealed(all credentials);
}
```

The `avoiding substrate` clauses are what make "the Leader decides,
the substrate acts" a checked fact: the Leader's verdict reaches the
genome only along the bus, through `dna::Dna`, and never because the
Leader holds a repository. The org adopts `Org` in its own main; the
application keeps whatever constitution it had.

## The record is seeded, not created

`init` does not write a file. It appends seven commits to
`refs/dna/journal`: what was attached (the entrypoint, the artifact's
digests, the toolchain), the structure the compiler observed (one
`structure.observed` per locus and topic, `provenance: observed`),
one proposed responsibility per locus (`ratified: false`), and the
purpose Review. From here every event is a commit on that branch —
[The record](./record.md).

## The two environments

```toml
[claims]
no_base = true

[environments.local]
source_only = true
entrypoints = ["."]

[environments.org]
source_only = true
entrypoints = ["dna/org"]
```

The application and the organization are two entrypoints under two
laws, and `hale check --matrix` checks both. `no_base` is stated
rather than inferred: an unstated shared base would be a rule that
looks bound and binds nothing.
