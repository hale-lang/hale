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
| `withdraw(host, render)` | what the render brought up on the host taken down again: the services it names, or its project's own when it names none |
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

The **`ssh` provider** places on the machines of the LAN, each a host
added by the name it was added under (`hale habitat host add <name>`),
resolved by the LAN's DNS and reached over plain SSH on its port (22
unless `--port`) as its user (`--user`, else the habitat's own): no DNS
and no TLS of the habitat's. It reaches every host with **the habitat's
own ed25519 keypair**, made into its vault the first time one is needed
(the slot `ssh-habitat-<id>`, `<id>` the habitat's record key; `ssh-keygen`
writes it into the local vault whole, and a vault at `HALE_VAULT_ADDR` is
provisioned out of band). The private key never leaves the vault: `ssh`
reads it by the vault file's path (a vault at `HALE_VAULT_ADDR` hands
`ssh` a private copy written on the sealed side, removed after the call),
and it appears in no output, log, row or error. `host add` prints the
public key, `ssh-ed25519 <key> habitat-<id>`, and the one line the host's
`~/.ssh/authorized_keys` takes, a step taken on that machine. **Host keys
are checked strictly**: the ed25519 key the host answers with when it is
added is kept in its row and shown with its fingerprint (trust on first
use), every connection trusts that key alone (a known-hosts file written
from the row, the host's name its alias, `StrictHostKeyChecking=yes`),
and a host that answers with another key is refused
(`host_key_changed`). What a host offers is probed over SSH: `compose`
when docker compose answers there, `systemd-user` when the user's systemd
does, and its OS and architecture. A host that does not accept the
habitat's key yet is `pending-key`, and `hale habitat host probe <name>`
moves it to `ready` once it does. The provider applies a render by copying
its files to `~/.hale/habitat/<project>/<render directory>` on the host
and running `docker compose -p <project> -f <file>… up -d --wait` there;
it withdraws one with `docker compose rm -s -f <service>…` (or `down` for
a render naming no service) and observes a service by `docker ps` there,
as the compose provider does here. It allocates the roles that run as
compose services (memory, the nerves, the senses' store, a heart's
database and broker); the habitat's head and face stay on `local`.

`up` records the habitat's placement once its record is up
(`habitat-place`): the host the compose provider produces, the `local`
profile's policy for its backends, and the habitat's own head and face
allocated and placed on that host, each a row written only when the record
does not already say it (below). The head's surface reads them back
(`spec/dna.md`, The head's surface).

## Nurseries and saplings

A **sapling** is an organism's seed in a nursery, an entry beside its
packs (`spec/dna.md`, Packs and the nursery): a repository pinned at a
revision with a digest, and the packs it adopts at graft. **The habitat's
nurseries are rows of its record**: the toolchain's bundled nursery,
always, and each directory or git repository its Board trusts, a
`habitat.nursery <source>` row written by the head's Board command
`Commands::add_nursery` (`hale habitat nursery add <source>`). A nursery
the habitat trusts is trusted for its tenants' `pack apply --from` too.

What the nurseries offer is read into the record by the habitat's acts:
`up`, `hale habitat nursery add` and `hale habitat nursery refresh`. The
first nursery to offer a name holds it (the toolchain's, then the
trusted ones). A sapling first seen is pinned, a `habitat.sapling <name>`
row. **An upgrade is a proposal, never a pull**: a revision or digest
other than the pinned one is filed as a `habitat.sapling.proposed <name>`
row, once, and nothing moves; `Commands::ratify_sapling` (`hale habitat
nursery ratify <name>`, the Board's) pins the filed revision, and an
organism already grafted stays at the revision it was grafted at.

`hale habitat graft <sapling>` takes a sapling by name, with no checkout:
a bare name, with no path of that name, that the record pins (`./<name>`
is always a directory). The pinned revision is fetched into the habitat's
own store, `<habitat>/saplings/<name>` (cloned once, its tags fetched
again for a graft that has not begun), its digest is computed from the
revision's tree and checked against the pin (a mismatch refuses and
nothing is grafted), and the repository there is grafted as the habitat's
child (`--attach`'s path). Its packs are proposed to the child's Board on
the holes its birth proposed, with the sapling's `set` and the graft's
`--set` values, each to the packs that take it. The graft's
`habitat.grafted` row names the `sapling`, `revision`, `digest` and
`nursery`. A graft that stopped part way goes on at the revision it was
checked at. Where the organism is placed is the graft's placement
argument, not built yet.

The toolchain's nursery offers **voice** (`hale-lang/voice`, pinned at a
commit). A sapling needs nothing beyond what an attached repository
needs: a `hale.toml` with its `[dna] needs`, and, for a heart migration,
a command that runs from a fresh fetch.

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

`hale habitat <verb>`; each verb prints its usage with `--help`. The host
verbs are clients of the habitat head's Board commands
(`Commands::add_host`, `::probe_host`, `::remove_host`, `spec/dna.md`):
they call the head's socket, so the habitat's body must be up.

| Verb | Behaviour |
|---|---|
| `up [--foreground \| --no-body] [--profile local] [--backends own\|operator]` | Births the habitat the first time, then brings every recorded allocation up through the `compose` provider, migrating any whose recorded schema version is behind this toolchain's, records its placement (its host, its policy, its head and face; above), and starts the habitat's body: a systemd user unit named for the habitat or, with `--foreground`, this process; and its face (below), a second unit or a child of this process. `--no-body` births it, brings its allocations up and records its placement, and starts no body. Idempotent. `--profile` takes `local`; any other is refused (exit 2). |
| `brief` | Prints the habitat's brief (below): the head's `Commands::brief`, asked over the head's socket as the person this account maps to. |
| `status` | The habitat, its genome, body and face (its address), its head's MCP endpoint while the face runs, and each allocation: its organism, whether each role's service runs, the schema version memory and the nerves record against this toolchain's, and how a heart's declared migration last ran. |
| `down` | Stops the habitat's face, its body and every service of its project. Data is kept; `up` brings them back. |
| `migrate [organism]` | Brings the organism's allocations up and migrates them: memory's schema and the nerves' stream move to this toolchain's, and the head's DSN is kept in the checkout's `.hale/dna/memory.env`. With `HALE_DNA_MEMORY_DSN_OWNER` or `HALE_DNA_NATS_URL_OWNER` set, that server is migrated instead. Idempotent; the organism defaults to the current directory. Runs the heart's declared migrations (below). |
| `sweep <root> \| --test-vaults [<root>]` | Takes down every compose project run from under `<root>`, a test's habitat, and confirms it gone by the project's label. Never the developer's habitat nor one outside `<root>`. Exit 1 names what survived. `--test-vaults` sweeps the vaults of processes that are gone. Nothing is done without docker. |
| `graft <dir> [--attach] [--no-library] [--no-pack] [--profile local\|remote-body [--remote <url>] [--body <user@host>]] [--with <pack> [--set <param>=<value>]… [--as <who>]]` | Births an organism as the habitat's child, the one way an organism other than the habitat is born. See below. |
| `graft <sapling> [--no-library] [--no-pack] [--set <param>=<value>]…` | Grafts a sapling the habitat pins, by name (above). |
| `nursery add <source> \| refresh \| list \| ratify <sapling>` | `add`: the Board trusts a nursery (`Commands::add_nursery`), then what it offers is read; `refresh`: every nursery read again; `list`: the nurseries and saplings (`Commands::nurseries`, `::saplings`); `ratify`: the filed revision pinned (`Commands::ratify_sapling`). |
| `host add <name> [--user <user>] [--port <n>]` | Adds a machine of the LAN as an `ssh` host (above): prints the host key first seen there and its fingerprint, what it offers, and the habitat's public key with the line its `~/.ssh/authorized_keys` takes. A host already added is refused (`host_exists`), and one where no SSH server answers is not added (`unreachable`). |
| `host probe <name>` | Probes the host again over SSH: a `pending-key` host whose key is now authorized is `ready`, with what it offers. A host that answers with another host key is refused (`host_key_changed`). |
| `host remove <name>` | Removes the host, a `habitat.host` row in state `removed`. Refused while any allocation is placed on it, naming them (`allocations_placed`); `local` is not removed. |
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

## Pairing: an agent beside a person

The head serves its surface, `HeadCommands`, over a third transport beside
its socket (`unix::Rpc`) and its commands port (`http::Rpc`): `mcp::Rpc`
(`spec/api.md` § The MCP transport), bound on the loopback at a port the
system gives it and reported with the others (`mcp=<port>` in the API
child's ports file, `HALE_DNA_API_PORTS`); `hale habitat status` prints it
as `mcp://127.0.0.1:<port>`. It is a binding, not a new path: the
exposure's bearer source and roles are the commands port's, so an agent
pairing with a person is a client of the head under that person's bearer,
the ID token the face's session holds under OIDC (the launch token in a
fixture's trusted-local session). There is no service account and no other
gate. `tools/list` answers a tool per row the person may call
(`Commands__hosts`, `Commands__add_nursery`, …); a row whose role they do
not hold is refused `unauthorized` (`-32005`) exactly as the commands port
refuses it (`403`). `hale mcp --app mcp://127.0.0.1:<port>` with the token
in `HALE_API_TOKEN` is the stdio bridge an agent's host runs.

Every habitat act is a row in the name of whoever asked: the rows
`Commands::add_host`, `::probe_host`, `::remove_host`, `::add_nursery` and
`::ratify_sapling` write are authored by the person the caller maps to, over
any transport. `agents/operator.md` is
the brief for the agent itself: the vocabulary, the rules, which rows are
reads, acts and proposals, and how to connect.

**The brief.** `Commands::brief` (`dna/api/habitat_brief.hl`), a read taking
`{limit}`, answers `{ok, why, revision, prompt}`: the Board's hat rendered as
a prompt with the live state. The hat part is the Board's position
(`position:board`) and its charter (the graph node's text, as
`dna/core/hat.hl` reads it), and the rules an agent works by. The live state
is what the other reads answer now: the hosts, the allocations with their
pulse (the state their provider observes), the organisms grafted, the
saplings pinned, and what waits for the Board: each Review requested and not
settled, with the authority it requires and its question, and each sapling
revision filed and not ratified. It reads the record and nothing else; no
vault is opened, so no secret value can reach it. `hale habitat brief`
prints it.

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
| `habitat.host <name>` | at `up` for `local`, and at `host add`, `host probe` and `host remove` for an `ssh` host, whenever what it says changes | `name`, `provider`, `offers` (space-separated), `state` (`ready`, `pending-key` or `removed`), `address`, `port`, `user`, `host_key` (the public host key first seen, `<type> <base64>`), `fingerprint`, `os`, `arch` |
| `habitat.policy <role>` | at `up`, whenever the role's provider changes | `role`, `provider`, `profile`, `backends` |
| `habitat.migrated <id>/<role>` | each time an organism role's schema version moves | `organism`, `role`, `from`, `schema_version` (memory's `MEMORY_SCHEMA_VERSION`, the nerves' `NERVES_STREAM_VERSION`) |
| `habitat.migrated <id>/<role>` | each run of a heart's declared migration | `organism`, `role`, `command`, `status`, `output` (a heart role carries no version) |
| `habitat.profile local` | at birth | `profile`, `backends`, `project` |
| `habitat.grafted <child>` | at a graft | `child`, `parent`, `needs`, `backends`, `name` (the directory), `face` and `head` (the organism's addresses), and for a sapling's graft `sapling`, `revision`, `digest` and `nursery` |
| `habitat.nursery <source>` | when the Board trusts a nursery | `source`, `kind` (`dir` or `git`), `state` (`trusted`) |
| `habitat.sapling <name>` | when a nursery's sapling is first read, and at its ratification | `name`, `purpose`, `repo`, `revision`, `digest`, `packs`, `set`, `nursery` |
| `habitat.sapling.proposed <name>` | when a nursery offers another revision of a pinned sapling, once | the sapling's fields at the new revision, and `from` (the pinned revision) |

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
