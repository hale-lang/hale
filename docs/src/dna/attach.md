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

`dna/org/main.hl` is one `main locus` with four children and its
connection to the nerves:

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
        nerves: nats::NatsConn = nats::NatsConn { url: dna::nerves_spine_url(), subject_prefix: dna::nerves_subject_prefix(), stream: dna::nerves_stream_here(), consumer: nats::ConsumerSpec { durable: dna::nerves_durable(), filter: dna::nerves_filter() }, … };
    }
    claims { adopt Org; }
    placement { nerves: pinned; }
    bindings {
        dna::ReviewVerdict: nats::NatsAdapter { };
        dna::IntentOffered: nats::NatsAdapter { };
        dna::ExpressionObserved: nats::NatsAdapter { };
        dna::PressureRaised: nats::NatsAdapter { };
        …
    }
    run() { while true { std::time::sleep(100ms); } }
}
```

`core` is the substrate: the record, the work system, the grant and
the policy, the Board as the membrane, the gateway, verification,
the editor. `leader` is the position that decides inside the grant.
`purpose` is the first Review. The bindings are the organization's
end of the **nerves** — the typed topics on which a verdict, an
intent, the host's observation report, a concern and a pressure
signal enter, over NATS; `nerves` is the connection that reads them
from the organization's stream ([the host, the nerves, the
nodes](./run.md#the-nerves)). Nothing decides in the transport; the
loci that own those topics decide.

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

## A repository with no application at its root

Some repositories are not one application: a seed per process, specs
they meet at, a compose file that runs them. Run `hale dna init` at the
root of one and it makes the organization there in the same way, with
the organization as the manifest's only environment, and seeds the
record with what the repository holds, as one graph: its purpose (the
README's first paragraph), its processes (the compose services), its
seeds (every manifest), its contracts (`spec/`) and the nouns they name,
its documents, its CI jobs as gates, its `FRICTION.md` entries as
witnesses. The last line of what it prints counts them by kind.

What a directory listing cannot say, you write in markdown, and init
reads it as written:

- **Mark a decision.** A list item that opens with a code span names
  its kind: `` `axiom` `` for a decision, `` `derived` `` for a
  consequence, `` `practice` `` or `` `law` `` for a rule about how work
  is done. Its bold sentence is its name, and the files an axiom links
  to are what it shaped.

  ```markdown
  - `axiom` **Only the api is exposed.** NATS and the nodes are on the
    private network ([`protocol.yaml`](./spec/protocol.yaml)).
  ```

- **Declare where things meet.** A table with `Served by`, `Consumed
  by` and `Over` columns says, for each contract it links, which
  process serves it, who consumes it and what it travels over; a table
  with `Deployment | Runs` columns names a deployment and what it
  runs. A name in code is a process or a seed, a link is a file, plain
  words are someone outside the repository.

  ```markdown
  | Document | Served by | Consumed by | Over |
  |---|---|---|---|
  | [`openapi.yaml`](./spec/openapi.yaml) | `api` | callers, `ui`, `node` | HTTP |
  ```

What the conventions cannot take stops `init` with the reason before
anything reaches the record: an axiom with no bold name, a table row
that links no contract, a name in code the repository does not have, a
name with a `|` in it. Fix the document and run `init` again; it keeps
the files it already made and seeds the record whole.

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
