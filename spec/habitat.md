# The habitat

This document specifies the habitat as shipped (GH #1653). It describes
the `local` profile, the only one built; the hosted profile is not. The
organism it serves is specified in `spec/dna.md`; the design it came from
is `docs/src/habitat.md`.

## What the habitat is

The habitat is a DNA organism the compiler ships, as it ships the
foundational law and the nursery packs. Its genome is fixed: its positions
are the nursery's `habitat` pack, its law the `standard` pack and its
genome document's `law` items. It is the one organism born without a
parent. **Every other organism is grafted from it**: its record's genesis
names the habitat's record as its parent.

The local habitat is one docker compose project per machine, named for the
habitat, with a service per allocation, under
`${XDG_DATA_HOME:-~/.local/share}/hale/habitat/local`. `HALE_DNA_HABITAT`
names that directory outright (a test's or a suite slice's habitat; the
developer's stays the default).

## Hosts, providers and placement

A **host** is a machine the habitat places on; the program `dna/host`
keeps its name, and `host` in a command name always means the machine.
Hosts are rows that providers produce, each declaring what it offers. A
**provider** is what allocates, places, applies and observes the
habitat's allocations on its hosts: every provider implements the core's
`Provider` interface (`dna/core/infrastructure.hl`):

| Verb | What it does |
|---|---|
| `offers(host)` | the host as the provider produces it: its name, the provider, what it offers (`compose`, `systemd-user`), or why it cannot be reached |
| `allocate(role, instance)` | a role (`memory`, `nerves`, `senses`, `database`, `broker`, `head`, `face`) of an instance (an organism's record key) allocated, on no host yet: the service that holds it and the offer its host must make |
| `place(allocation, host)` | the allocation placed on the host, or refused, naming the offer the host lacks |
| `apply(host, render)` | a render applied on the host; until a fleet's render exists, a render is what `hale deploy render --backend compose` writes (a compose project, its directory, its files and the services to bring up) |
| `observe(allocation)` | `up`, `degraded` (running, not well) or `down`, and what was seen |

**Placement is the record's.** Every allocation is a row naming its role,
its instance, its provider and its host; a source's placement is a default
only. A **policy** maps each role to the provider that allocates it, per
habitat, and a **profile** is a preset of policy: `local` gives the
`compose` provider every role, and on `operator` backends the operator's
servers hold memory, the nerves and a heart's database (the provider
`operator`: adopted, never applied, and not observed by the habitat), with
no `broker`.

The **`compose` provider** is local only. It produces this machine as the
host `local`, offering `compose` when docker compose answers and
`systemd-user` when the user's systemd does. Memory, the nerves, the
senses' store, a heart's database and its broker need `compose`; the
habitat's head and face, one process of the habitat's (`hale dna head`),
need no offer. It applies a render with `docker compose -p <project>
--project-directory <dir> -f <file>… up -d --wait [<service>…]`, with
`COMPOSE_IGNORE_ORPHANS=1`: every allocation the habitat brings up, the
habitat's own and each tenant's, is brought up this way. It observes a
service by its container's state (`running` is up, `running` and
unhealthy or `restarting` is degraded, anything else or no container is
down) and the head and the face by whether their address accepts a
connection. A provider in memory (`MemProvider`) stands in for it in tests.

`up` records the habitat's placement once its record is up
(`habitat-place`): the host the compose provider produces, the `local`
profile's policy for its backends, and the habitat's own head and face
allocated and placed on that host, each a row written only when the record
does not already say it (below). The head's surface reads them back
(`spec/dna.md`, The head's surface).

## Long-lived, and only its acts provision

- Nothing tears the local habitat down when work ends. Organisms are
  tenants reused from run to run; two checkouts of one organism are the
  same tenant.
- Provisioning and migration are habitat acts, and only those: `up`,
  `graft`, `migrate`, and `hale dna upgrade` for an operator's own
  memory. Nothing migrates mid-work: `hale dna dev`, `run`, `head`, `show`
  and `work` and the host's verbs run on what is up and migrated, derive
  the roles' endpoints from it without provisioning anything, and, when
  something is missing or behind, say so and name the act that supplies
  it (`hale habitat migrate <organism>`).
- Only a test's habitat is taken down (`sweep`).

## Verbs

`hale habitat <verb>`; each verb prints its usage with `--help`.

| Verb | Behaviour |
|---|---|
| `up [--foreground \| --no-body] [--profile local] [--backends own\|operator]` | Births the habitat the first time, then brings every recorded allocation up through the `compose` provider, migrating any whose recorded schema version is behind this toolchain's, records its placement (its host, its policy, its head and face; above), and starts the habitat's body: a systemd user unit named for the habitat or, with `--foreground`, this process; and its face (below), a second unit or a child of this process. `--no-body` births it, brings its allocations up and records its placement, and starts no body. Idempotent. `--profile` takes `local`; any other is refused (exit 2). |
| `status` | The habitat, its genome, body and face (its address), and each allocation: its organism, whether each role's service runs, the schema version memory and the nerves record against this toolchain's, and how a heart's declared migration last ran. |
| `down` | Stops the habitat's face, its body and every service of its project. Data is kept; `up` brings them back. |
| `migrate [organism]` | Brings the organism's allocations up and migrates them: memory's schema and the nerves' stream move to this toolchain's, and the head's DSN is kept in the checkout's `.hale/dna/memory.env`. With `HALE_DNA_MEMORY_DSN_OWNER` or `HALE_DNA_NATS_URL_OWNER` set, that server is migrated instead. Idempotent; the organism defaults to the current directory. Runs the heart's declared migrations (below). |
| `sweep <root> \| --test-vaults [<root>]` | Takes down every compose project run from under `<root>`, a test's habitat, and confirms it gone by the project's label. Never the developer's habitat nor one outside `<root>`. Exit 1 names what survived. `--test-vaults` sweeps the vaults of processes that are gone. Nothing is done without docker. |
| `graft <dir> [--attach] [--no-library] [--no-pack] [--profile local\|remote-body [--remote <url>] [--body <user@host>]] [--with <pack> [--set <param>=<value>]… [--as <who>]]` | Births an organism as the habitat's child, the one way an organism other than the habitat is born. See below. |
| `handover [organism]` | Prints what the habitat hands the organism's heart, one `NAME=value` a line. |

The free-standing birth verbs are retired (`spec/dna.md`, Graft): each
exits 2 naming the graft that replaces it.

## The face

The habitat is served as every organism is: the standard face over its own
head (`spec/dna.md`, The habitat's face). `up` starts `hale dna head` on the
genome, its state under `face-state` in the habitat's directory. The
habitat's address, and each organism's, is a block of 8 loopback ports
recorded in a `dna.head.url` (the habitat's in its genome's, an organism's
in its own, assigned at its graft); the face is the block's base and the head
the next port. The face's index lists the organisms the record grafted, with
their state and the addresses of their faces and heads. A habitat started
under `HALE_DNA_TRUSTED_LOCAL=1` serves its face without the stub provider,
as a fixture's head does.

The genome carries a `ui/` (`dna/habitat/ui`), written at birth and tracked,
which the standard face serves under `/app/` as it serves any attached
project's: the operator face. Its first screen shows the sign-in edge and
the habitat's name, state, record revision and organisms, read by the
page from the head's `Commands::index` row through the generated ES-module
client committed beside it (`head-client.js`, from `dna/face/operator`). It
has no build step. A visitor with no session is redirected by the head's
gate before the page is served; a read refused for want of one is shown as
the crossing. The page's visual rules are CSS custom properties
(`operator.css`): near-black ground, one monospace, hairline rules, brass
only for a crossing, color only for state (up, degraded, down).

## Backends

Chosen at the habitat's birth and kept.

- `own` (the default): memory, the nerves and the senses' store are
  allocations of the habitat's compose project.
- `operator`: the Postgres and the NATS named by
  `HALE_DNA_MEMORY_DSN_OWNER` and `HALE_DNA_NATS_URL_OWNER` are adopted,
  and the habitat provisions none of its own. A role that needs server
  configuration of the operator's NATS (`broker`) is refused at graft on
  `operator` backends.

## Graft and `[dna] needs`

A graft allocates each role the organism's genome needs, records the
allocations in the habitat's record, writes a `habitat.grafted <child>`
row (`child`, `parent`, `needs`, `backends`) and proposes the `standard`
pack to the child's Board (`--no-pack` leaves it for `hale dna pack apply
standard`). `<dir>` is a new application's directory or, with `--attach`,
an application or repository already there. A graft that stopped part way
goes on where it stopped; a record born of anything but this habitat is
refused.

The genome declares its roles in its `hale.toml`:
`[dna] needs = ["memory", "nerves", "senses", …]`.

- Memory and the nerves are always among them; the senses' store unless
  left out. An undeclared `needs`, as a foreign heart's, is exactly these
  organism roles, never a heart role.
- A Hale heart may declare heart roles. **`database`**: a tenant database
  of the organism's memory server, `heart_<id>`, owned by a role of the
  same name that it alone reaches, its password in the vault slot
  `postgres-heart_<id>`; provisioned on either backend. **`broker`**: two
  users of the organism's own nerves server, `heart-<id>` and
  `heart-<id>-node`, confined to the subject space `heart.<id>.>`, their
  passwords in the vault slots `nats-<org>-heart` and
  `nats-<org>-heart-node`; requires `own` backends.
- `cache` and `object-store`, and any role the habitat does not know, are
  refused by name.

## Allocation rows

Every provisioning act is a row of the habitat's record, keyed by the
organism and the role (`<id>/<role>`), never by a checkout.

| Row | When | Body |
|---|---|---|
| `habitat.allocated <id>/<role>` | the first time a role is provisioned for an organism; for the habitat's own `head` and `face`, at `up` whenever their placement changes | `organism`, `role`, `service`, `project`, `provider`, `host`, `vault` (slots held by name, never a value), and for the head and the face their `address` (`""` while none is assigned) |
| `habitat.host <name>` | at `up`, whenever the host or its offers change | `name`, `provider`, `offers` (space-separated) |
| `habitat.policy <role>` | at `up`, whenever the role's provider changes | `role`, `provider`, `profile`, `backends` |
| `habitat.migrated <id>/<role>` | each time an organism role's schema version moves | `organism`, `role`, `from`, `schema_version` (memory's `MEMORY_SCHEMA_VERSION`, the nerves' `NERVES_STREAM_VERSION`) |
| `habitat.migrated <id>/<role>` | each run of a heart's declared migration | `organism`, `role`, `command`, `status`, `output` (a heart role carries no version) |
| `habitat.profile local` | at birth | `profile`, `backends`, `project` |
| `habitat.grafted <child>` | at a graft | `child`, `parent`, `needs`, `backends`, `name` (the directory), `face` and `head` (the organism's addresses) |

An allocation names its provider and its host: `compose` on `local`, or,
on `operator` backends, `operator` and no host. The latest row for an
entity is the one that holds. An allocation made before the habitat's record exists waits in
`allocations/<id>/allocation.json`, the local profile's cache of the rows,
and is written at birth. `status` reads the rows, and the cache only while
the habitat's memory is not up.

## Opt-in heart migrations

A heart owns the schema of a heart role it declares. It may hand the
migration to the habitat, in its manifest:

```toml
[dna.migrations.database]
command = ["./migrate", "up"]
```

The `command` is the argv of the heart's own tool, run in the organism's
directory with the role's handover and the rest of the attach handover in
its environment, at every graft, `hale habitat migrate` and `hale dna
upgrade`. The tool is idempotent and keeps its own ledger; the habitat
keeps no version of the heart's schema. The role's allocation is brought
up and waited for first. A failing run fails the act that ran it, naming
the command, and the next act runs it again. A malformed declaration, a
key other than `command`, or a role the genome does not need is refused by
name. A heart that declares none owns its schema however it likes; the
habitat migrates nothing of a heart's on its own.

## The handover

What the habitat hands a heart, a secret only ever by the name of its
vault slot. `hale habitat handover` prints it; `hale dna dev` hands the
same set to the host, which the heart inherits.

| Variable | Value |
|---|---|
| `HALE_DNA_APPLICATION_ID` | the record's identity |
| `HALE_DNA_HEAD_URL` | the head the heart reaches (`dna.head.url`; `http://127.0.0.1:8792` by default) |
| `HALE_DNA_OIDC_ISSUER` | the issuer (`dna.oidc.issuer`; `http://127.0.0.1:8794` by default) |
| `HALE_DNA_OIDC_CLIENT` | the heart's service client, mapped in the child's record at graft |
| `HALE_DNA_OIDC_CLIENT_VAULT` | `oidc-service-<client>`, the slot of its secret |
| `HALE_DNA_DATABASE_DSN` | for an allocated `database`: a DSN naming the role `heart_<id>` and its vault slot, no password |
| `HALE_DNA_BROKER_URL`, `_USER`, `_VAULT`, `_SUBJECTS`, `_NODE_USER`, `_NODE_VAULT` | for an allocated `broker`: the nerves server (no credential), the heart's user and its slot, its subject space `heart.<id>.>`, and the nodes' user and slot |

The manifest is read once, by graft; every reader after it reads the
roles the habitat allocated, as `allocation.json` keeps them.
