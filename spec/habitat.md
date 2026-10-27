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

**A habitat has a name**, given at its birth with `hale habitat up --name
<name>`: a DNS label (1 to 63 lowercase letters, digits and `-`, neither
first nor last a `-`); `local` when its birth names none. The name is
recorded on its `habitat.profile` row and kept: an `up` that asks another
name of a habitat born is refused (exit 2). `hale habitat status` shows it
(`name` line), the head's `Commands::index` answers it (`name`, `""` for
any record but a habitat's), and the operator face's first screen shows it
as the habitat's name.

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
host `local`, offering `compose` when docker compose answers. Memory, the
nerves, the senses' store, a heart's database and its broker, and the
habitat's own `body` (`hale dna dev <genome> --no-iris`) and `face`
(`hale dna head <genome>`) run as services of the habitat's compose project,
so a host runs nothing outside docker. For `body` and `face`, `up` writes
the toolchain's embedded `Dockerfile` and `hale.pin` (defaulting to the
commit the running toolchain was built from) beside `compose.yaml` in the
habitat's allocation directory (`allocations/<key>/`), building the image
from `https://github.com/hale-lang/hale.git#<pin>` unless `--image <tag>` or
`HALE_HABITAT_IMAGE=<tag>` (flag over environment) names an existing local
image (`hale habitat image build [<checkout>]` builds it once from a local
checkout; the `Dockerfile`'s `HALE_FROM=artifact` packages a `hale` and
`libhale_ts_shim.a` built already instead, as CI does). Both services mount
the habitat directory, the local vault directory and the host's docker
socket, with the head's HTTP surface and the face published on their
loopback ports. It applies a render with `docker
compose -p <project> --project-directory <dir> -f <file>… up -d --wait
[<service>…]`, with `COMPOSE_IGNORE_ORPHANS=1`: every allocation the habitat
brings up, the habitat's own and each tenant's, is brought up this way. It
observes a service by its container's state (`running` is up, `running` and
unhealthy or `restarting` is degraded, anything else or no container is
down) and the head and the face by whether their address accepts a
connection. A provider in memory (`MemProvider`) stands in for it in tests.

An allocation on this machine is recorded where it is published,
`127.0.0.1:<port>`, and each process derives the address it dials from that
record and where it runs (`dna::reach_url`, `dna::loopback_host`); nothing
rewrites a connection as it is made. On Linux `body` and `face` share the
host's network (`network_mode: host`), so the recorded address is theirs as
it is the admin machine's. On macOS, where a container cannot share the
host's network, they run on the project's network and the render sets
`HALE_DNA_LOOPBACK_HOST=host.docker.internal` on both: a port published on
the machine's loopback is dialled there. A listener in a service's own
container (the face's head and its API child, the OIDC stub) is dialled at
`127.0.0.1` in both.

Nothing the habitat runs binds every interface of the host. The face and
the head, the listeners a person reaches, bind the address the habitat
declares (`HALE_DNA_HABITAT_LISTEN`: `127.0.0.1` for a local habitat, the
host's interface for one on another machine, below), handed to the `face`
service as `HALE_DNA_LISTEN_HOST`, and so does the OIDC stub; the head's
API child binds `127.0.0.1`. The stub's issuer and endpoints, the head's
origin and the `Host` it answers as are recorded under the habitat's
declared address (`HALE_DNA_HABITAT_ADDRESS`, handed to `body` and `face`:
`127.0.0.1` locally, `<name>` on another machine), so a browser on the
admin machine signs in at the face it reaches; plain http is admitted for
the loopback and exactly that address, no other host (`spec/dna.md`
§ OIDC). On macOS the face, the head and the stub bind `0.0.0.0` inside
the `face` container, on the project's network, and compose publishes
their ports on the declared address alone.

### A habitat on another machine

`hale habitat up --host <name> [--user <user>] [--port <n>]` runs the
habitat on a machine of the LAN that has docker and nothing else. The
admin machine reaches it over SSH as the person's own login (`--user` and
`--port`, else the SSH config's; `HALE_HABITAT_SSH_CONFIG` names a config
file for `ssh -F`), never with the habitat's keypair, which is drawn there,
in the habitat's vault, as a local `up` draws it. The toolchain image is put
there first: there already; sent from the admin machine's docker (`docker
save | docker load`); for a release's build (the tag `v<version>` on
`hale-lang/hale` names the toolchain's own commit), pulled as that
release's published image when its `hale --version` answers as the
admin's does; else built there from the toolchain's commit on
`hale-lang/hale`, the source a local habitat's compose renders as its
`build:`, which needs that docker's `buildx` plugin and the commit pushed.
A build from any other commit shares the version string and never pulls
the release's image. An image of a platform other than the machine's
docker (`<os>/<arch>`) is never used or sent. Then the same `hale habitat up` runs there in
that image, as the login's user, on the host's network, with the user's
data directory (`~/.local/share/hale`: the habitat, its genome, record
and vault) and the docker socket mounted, in the socket's group as the
daemon has it (read by a `stat` in the same image with the same mount,
since the login's shell may see another socket, as a VM's docker on
macOS has), and with
`HALE_DNA_HABITAT_LISTEN` (the address `<name>` resolves to on the admin
machine; `0.0.0.0` on a machine whose login shell says `Darwin`, since
its docker runs in a VM that owns none of the machine's addresses and
publishes what listens on the VM's interfaces on the machine's own) and
`HALE_DNA_HABITAT_ADDRESS` (`<name>`): the face and head bind that
address and are recorded at `http://<name>:<port>`. The admin machine
waits on the head and the face at the recorded URL, then keeps a pointer in
its local habitat's directory in place of a habitat: `remote.json` (the
name, the host, the login, the image, the head's and face's URLs, the
record's identity), `head.url`, and the head's launch token
(`face-state/head.token`, 0600). `up` and `status`, local or through a
pointer, end with a `sign in` line while the face answers: the face's URL
with that token (`/?token=…`), the link that opens the face once and sets
its session, which is otherwise printed only in the face's own log. A machine points at one habitat, and one
that runs a habitat of its own points at none.

Every verb on the admin machine reaches the habitat's head through the
pointer (`--head`, `HALE_DNA_HEAD_URL` and `head.url` resolve as they do
locally). `status` prints the pointer and asks the head
(`Commands::index`, `Commands::allocations`). `down` and `down --forget`
run there over SSH, in the image, only on the record the pointer names:
the identity there is read first, and the run there checks it again
(`HALE_DNA_HABITAT_EXPECT`), so a habitat made there again from another
machine is never stopped or removed through a stale pointer. `--forget` plans there and is confirmed
there by the habitat's name typed at the person's terminal, which SSH
carries (refused off a terminal), and the pointer is removed last, once
the habitat there holds no record. `graft <dir>` runs there the same
way, after the same check of the record: a directory on the admin machine
is shipped there whole (`tar | ssh`) as the habitat's own copy,
`<habitat>/grafted/<name>`, which is the organism's home from then on
(its removal and the habitat's forget delete it, as they delete a
sapling's clone), and a path that does not exist is a new application
made there; every flag is passed through. Each graft ships into a
directory of its own, `<habitat>/grafted/<name>-<suffix>`, made fresh
there (`mkdir`, never `-p`) before anything is unpacked, so nothing is
unpacked over another organism's home. A graft that fails removes that
directory only when it answered and left no mark of a graft (a pinned
record, `.hale/dna`); otherwise the directory is kept and the command
that goes on from it is printed, `hale habitat graft --resume <path>
[flags]`: the same graft run on that directory, nothing shipped, which
takes only a directory under `<habitat>/grafted/` that exists there. The organism's own verbs (`hale dna review`,
`task create`, `work run`, `show`, …) run in its directory there through
`hale habitat organism <name> dna <verb> [args…]`, at the person's terminal
when there is one; locally the same verb runs them in the organism's
directory here. `migrate` and `handover`, which act on a genome on the
admin machine and have no route through the head, are refused while the
pointer names another machine.
`up --host` again finds the habitat there by its record's identity (one
whose identity differs is refused, nothing changed), runs the same
idempotent `up`, which changes nothing, and writes the pointer again.
`up` without `--host` on a machine that points elsewhere is refused.

`body` and `face` mount the host's docker socket because the habitat is the
compose provider from inside its containers: `body` brings up, restarts,
observes and probes the allocations of every organism the habitat carries
and execs into them to provision their credentials, and the head reads
their published ports. Access to that socket is root on the host: whoever
can run code in either service (the toolchain image, the habitat's genome,
the organisms grafted under it) can start any container with any host path
mounted. A habitat is therefore trusted as the machine's administrator is,
and an organism runs under it only by graft.

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
`~/.ssh/authorized_keys` takes. **The habitat logs in as an account of its
own**, `hale-<id>` (`<id>` the record identity's first 12 hex digits),
never as the person, unless `--user` names an account: one per habitat, so
two habitats on one host never share a login. **The CLI sets it up when
the person's own SSH login reaches the machine** (as `up --host` reaches
one: their SSH config, `--port`, `HALE_HABITAT_SSH_CONFIG`): with their
sudo, asked at their terminal, it makes the account (a system user with
its own home and no sudo, in the `docker` group, which is root-equivalent
on the host: the account keeps the habitat out of the person's account,
not out of the host), puts the habitat's key in its `authorized_keys`, and
probes the host again, so `hale habitat host add <name>` ends `ready`. A
host with no `useradd` (a Linux host is needed) is refused by name. With
`--user`, the line is appended to that account instead. When the login
does not reach the machine, what to do there is printed. `hale habitat
host remove` and, for a habitat on another machine, `hale habitat down
--forget` undo it the same way: the habitat's account removed with its
home, or, for an account `--user` named, the line taken off by its
comment (`habitat-<id>`), every other line kept; a host the login does not
reach keeps it, and the CLI says what to remove. The operator face, which
cannot reach a machine, shows the line to copy. **Host keys
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

### Placing and applying

**"Change a host"** is the Board's command `Commands::place_allocation`
(`{allocation, host}`, `hale habitat place <key>/<role> <host>`): a
`habitat.allocated` row naming the host, the host's provider and the
compose project on it (the habitat's own on `local`,
`<habitat project>-<host>` elsewhere). Nothing applies. The roles a
placement moves are memory, the nerves, the senses' store and an
organism's own services; a heart's database and broker move with the
server that holds them, the habitat's head and face run with its body,
and an allocation on the `operator`'s server is never placed.

**Apply** is the Board's command `Commands::apply_allocations` (`{host,
fresh}`, `hale habitat apply [<host>] [--fresh]`), over every allocation
or those placed on or leaving one host:

- the allocation rows are exported as the `--placement` file `hale deploy
  render` takes (`<habitat>/placement.json`, `{instance, host, provider,
  role}`);
- each allocation that moved is withdrawn where it was last applied, by
  that host's provider, and brought up where it is placed (`Provider.apply`:
  compose here, ssh elsewhere), and where it was applied is kept beside its
  files (`allocations/<key>/placed.<role>`, the local profile's cache of the
  rows) for everything that derives an address from it;
- what each provider observes after it is the pulse, printed per
  allocation, and each apply is a `habitat.applied <key>/<role>` row
  (`host`, `provider`, `ok`, `why`, `state`, `fresh`).

**Derivation follows placement.** On `local` an allocation's address is the
loopback port `docker compose port` gives. On any other host it is the
host's name, resolved by the LAN's DNS, and the port the allocation's
`ports.yaml` chose, published there on the host's interface
(`ports.remote.yaml`, written beside it). No TLS: the LAN is plain. The
owner's DSN, the head's (`.hale/dna/memory.env`) and the nerves' URL are
derived that way, and a password reaches the server only through the
secrets file compose mounts, copied beside the render to the host, never in
a render or a row; the password adoption runs inside the container there.

**A moved memory or nerves allocation starts empty on its new host**: there
is no migration. A memory that was applied holds its organism's record, so
its move is refused (`memory_holds_rows`) unless the apply is `--fresh`,
which forgets what the new host held for it (its volume) and brings it up
empty. The habitat's own memory holds the record that places everything,
and is not moved. After a move, whatever reads the address picks it up on
its next start: the apply restarts the habitat's own face and head when its
nerves moved, and an organism whose memory or nerves moved derives the new
address at `hale habitat migrate <organism>` (a memory started empty holds
no record, and the organism is grafted again).

**Graft takes a placement**: `hale habitat graft <sapling> --host <name>`
places the new organism on that host before anything of it is brought up.
Its memory, nerves and senses' store are brought up there by its birth,
which derives their addresses from there, so its record is born in the
memory there. Its own services (a `compose.yaml` at its root) are an
allocation `<key>/services` of that host (its `render_dir`,
`render_files` and `render_mounted`: each entry the repository tracks at
its root but its `compose.yaml` and `hale.toml`, a file or a directory),
applied there through the head. The `ssh` provider ships a render whole,
its compose files and each mounted entry (a directory with everything
under it: a build context, a bind mount's source), as one tar extracted
into `~/.hale/habitat/<project>/<dir>` on the host, with no access for
anyone but its user. What the services
publish is read by the provider where it applies them, from compose's own
reading of the render there with the environment it applies it with (the
host's facts, below): `docker compose config --format json`, each service
with a TCP port published, its first, as `{service, port, bind}` (`bind`
the address it is bound to, "" for every interface). Each apply records it
on its `habitat.applied` row (`published`), at the graft's first apply, at
a later apply and at a move alike; an apply or a withdrawal that fails
records none and leaves the last successful one in force (one that takes
and publishes nothing, `published: []`, replaces it). `Commands::allocations`
resolves the latest successful one against the host the allocation
actually runs on and answers the
allocation's `links` (`{service, url, note}`), a link only where it is
reachable from the admin machine: on another host, a port bound to every
interface at the host's address, one bound to a specific address at that
address, and one bound to the host's loopback with no `url` and the note
`loopback only on <host>`, unless the host's listen address is itself the
loopback and the habitat is local too (it declares no address,
`HALE_DNA_HABITAT_ADDRESS`, or the loopback: the head that resolved the
host runs where the browser opens the link), where it is linked at
`127.0.0.1`; under a habitat on another machine such a port keeps the note,
since the head's resolution says nothing of the browser's machine. On
this machine at `127.0.0.1`, or at the address it is bound to, under a
local habitat; under a habitat on another machine, the head's own machine
(its `local` host, or a host whose address resolves to the loopback there)
is reached at the habitat's declared address: a port bound to every
interface links there, one bound to the loopback gets the note. The
operator face lists them under the organism's
allocation, a link where there is a `url` and the note where there is
none: voice's `api`, whose `/` is its console, among them. Every allocation names its
provider: the host's. A tenant's model map is the head's Board command
`Commands::set_model_map` (`{organism, category, model, endpoint,
key_vault, tools}`, `hale habitat model-map`): the organism's `hale dna
models category … --endpoint <url> [--key-vault <slot>]`, run in its
directory, a key only by its vault slot's name; `Commands::probe_model`
(`{organism, category}`, `hale habitat model-probe`) asks the model the
category names, where the map says, once, and answers what it said, never
a key. A leg's Work on a mapped category is asked the same way: the model
router (`ModelRouter`, `dna/core/models.hl`) sends a request that names an
endpoint there, with that endpoint's key slot or none, whichever backend
the organism's catalog would have chosen, and a key slot the vault lacks is
refused there, never answered by another backend. A sapling's graft is the head's Board command
`Commands::graft` (`{sapling, host, library}`): it starts the act, which
takes minutes, and answers at once with its log (`<habitat>/grafts/<name>.log`,
its exit status beside it in `<log>.exit`); the end of the graft is its
`habitat.grafted` row. `hale habitat graft <sapling>` is a client of it while
the head answers, following the log to the exit, and the act itself
otherwise or with `--no-head` (what the head runs).

The habitat's own infrastructure as a fleet seed, rendered by `hale deploy
render --placement`, is not built. That renderer renders a fleet's Hale
programs, while the habitat's memory and nerves are images whose passwords
come from the vault and its body and face are services of its compose
project built from the toolchain's Dockerfile. The exported `placement.json`
is the seam a render takes, and until then each host is handed the render
each allocation already has (its own compose files, or an organism's
`compose.yaml`).

`up` records the habitat's placement once its record is up
(`habitat-place`): the host the compose provider produces, the `local`
profile's policy for its backends, and the habitat's own head and face
allocated and placed on that host, each a row written only when the record
does not already say it (below). The head's surface reads them back
(`spec/dna.md`, The head's surface).

## Forge and identity

**The forge is a gate, not a store.** Code goes out through it as
branches, and verdicts come back through it; the organism reads nothing from
it that it did not already decide. A Review lives in the record, its
candidate is a branch in git, and the pull request is a mirror. Which forge
stands in the gate is an allocation, and swapping it is swapping a remote.

`forge` and `identity` are roles the habitat allocates, as it allocates an
organism's memory: each a `habitat.allocated <id>/forge` and
`<id>/identity` row naming its provider, written once, when absent, at `up`
for the habitat and at a graft for the organism (a retried graft records
what an interrupted one left out, and never overwrites an allocation the
Board made since). The identity is `oidc-stub`, the habitat's own sign-in,
at the organism's issuer (`dna.oidc.issuer`, else the stub's default
address), which `Commands::allocations` observes by whether it answers.

**The `local` profile's forge is Forgejo**, the habitat's own: on its own
backends the compose provider brings one Forgejo up at every `hale habitat
up`, as it brings up memory and the nerves. It is a service (`forge`) of the
habitat's compose project, its image pinned by digest, published on a
loopback port chosen once (`<habitat>/forge/port`), its data in the
project's volume. Its administrator (`hale-admin`) has a password drawn
into the vault the first time (`forgejo-admin`); the server reads it from
the file compose mounts, inside the container, never from a command line,
and the habitat's forge row names the slot, never the value. On an
operator's backends the policy's forge is `none` until the Board allocates
one. A graft gives the organism a Forgejo user of its own, named for it and
its key (`<name>-<id>`, so two organisms never share an account however
alike their names), its token drawn into the vault slot `forgejo-<id>`,
and a repository for its codebase (`<name>-<id>/<name>`); its forge row names the provider
`forgejo`, the address, the repository and the slot. `hale dna forge sync`
keeps the organism's branch there as it stands (a branch the forge moved
past, by a merge there, is left as the forge has it), pushing as the
organism's user with its token handed to git through a credential helper
reading a private file.

The forge's implementations sit behind the core's `Forge` interface:
`ForgejoForge` (Forgejo's API, which maps onto GitHub's nearly one to one:
a pull request per review, its approving or change-requesting reviews as
verdicts, the current status of each context at the candidate as its
check runs (a pass before a failed or pending rerun is never one), a
comment at settlement),
GitHub through `gh`, the file forge (the fixtures'), and `NoForge`, a bare
remote whose Reviews are decided in the organism's own face or CLI, the
no-infrastructure fallback. **GitHub is the allocation when a Board names
it**; neither GitHub nor `none` is the default. A graft takes `--forge
<provider>` over the policy. The Board allocates a forge with
`Commands::allocate_forge` (`{organism, provider, repo, key_vault}`, `hale
habitat forge <organism> forgejo|github|file|none [--repo <owner/name>]
[--key-vault <slot>]`; `habitat` names the habitat's own): Forgejo makes the
organism's user and repository there as a graft does; GitHub names its
repository and the vault slot of its token (`forge-token` by default), never
the token; `file` is a forge of files. The host reads the allocation from
its cache beside the organism's allocations (`allocations/<id>/forge.json`)
at each sync; `HALE_DNA_FORGE` stays the fixtures' override.

`hale dna forge sync` is the forge's pass, against whichever forge the
allocation names, as `github sync` was: a pending mutation Review is opened
as a pull request (its candidate pushed as `dna/<review>`), the forge's
verdicts and check runs come back as rows, a settlement goes back as a
comment, and a leg's review at an attached codebase's forge comes back as
`task.delivered` when merged or `task.refused` when closed, its comments as
receipts. The `github.pr` row names the forge it was opened at, and only the
rows of the forge in the gate now are read: moving a project from one forge
to another is changing the remote, changing the allocation, and syncing. A
Review still open reappears as a pull request at the new forge; nothing
migrates, and the old forge's pull request is left as it was.

No practice, workflow or standard names a forge by kind: "open a review" is
the verb, and the allocation answers it (a guard test reads every pack). A
leg's hands are the same: their forge is `AllocatedForge`
(`dna/core/legs/hands.hl`), which reads the organism's allocation at each
call, as forge sync does, and opens the leg's review at the allocated
Forgejo (the codebase's repository there, by its url), through `gh` for
GitHub, or refuses with `none`; the git hand (`ScratchGit`) pushes a
leg's branch to a repository at the allocated Forgejo as the organism's
own user, its token handed to git through a credential helper reading a
private file. The guard refuses a `Hands` whose forge
names a kind.

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
first nursery to offer a name holds it (the trusted ones, then the
toolchain's, which is the default a trusted one may offer over). A sapling first seen is pinned, a `habitat.sapling <name>`
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
child (`--attach`'s path). **A sapling the toolchain carries** names no
remote: its repository is `toolchain:<path>`, a tree of the toolchain's own
embedded nursery (`dna/nursery/<path>`, shipped as its packs are), which
only the toolchain's nursery may offer. Its graft writes that tree into the
store, makes it the organism's own repository there (one commit, no
remote), and checks the commit's tree against the same digest a fetched
sapling is pinned by; a tree that is not the pinned one is refused, the
store removed, and nothing grafted. Its revision is `embedded`. Like every
graft, it mirrors the organism's repository to the forge the habitat
allocates it (the habitat's Forgejo on its own backends, `spec/dna.md`):
nothing of an organism's own is on GitHub unless its Board allocates
GitHub later. Its packs are proposed to the child's Board on
the holes its birth proposed, with the sapling's `set` and the graft's
`--set` values, each to the packs that take it. **A sapling's packaging is
ratified by its graft**: the person who grafts it chose it, so the graft
then decides every pending group in their name (`hale dna review all
approve --no-wait`, its verdicts executed when the child's body runs) and
says `ratified its purpose, practices and positions`. So does the
habitat's birth for the toolchain's foundational positions and law (the
`habitat` and `standard` packs). Anything proposed after is the Board's to
review, and a directory grafted as a new application keeps its proposals:
no one vetted that packaging. A ratification that fails is said, with the
command that finishes it. The graft's
`habitat.grafted` row names the `sapling`, `revision`, `digest` and
`nursery`. A graft that stopped part way goes on at the revision it was
checked at. Where the organism is placed is the graft's placement
argument, not built yet.

The toolchain's nursery offers one sapling, **the hale organism** (`hale`,
a tree the toolchain carries): the organism that makes the toolchain. An
application is not bundled. **A repository can be its own nursery**: a
`nursery.toml` at its root whose sapling says `repo = "."` and no revision
or digest offers the repository itself, pinned where the nursery is read
(a git nursery at the commit it was read at, a directory at its `HEAD`)
by that tree's digest; a later read at a newer commit is a proposal, as for
any sapling. The toolchain's nursery refuses that form. `hale-lang/voice`
is one: `hale habitat nursery add https://github.com/hale-lang/voice` (or
the organisms screen's form) trusts it, and `hale habitat graft voice`
grafts it. A sapling needs nothing beyond what an attached repository
needs: a `hale.toml` with its `[dna] needs`, and, for a heart migration,
a command that runs from a fresh fetch.

**The hale organism** is the organism that works on the Hale toolchain.
Its genome is the tree the toolchain carries at `dna/nursery/saplings/hale`
(its `hale.toml` with `[dna]`, its main, a README), pinned by that tree's
digest, with no repository of its own anywhere else: `hale habitat graft
hale` makes it the organism's repository in the habitat and mirrors it to
the habitat's Forgejo. The code it works on is its codebases, other
repositories its manifest lists, `hale-lang/hale` and `hale-lang/voice`; its
positions and their mandates are the nursery's `hale-organism` pack, which
the sapling adopts at graft. It is neither the compiler's repository as a
sapling nor the habitat's own organism.

**A genome's codebases attach at its graft.** `[[dna.codebase]]` in its
`hale.toml` lists them with phase 3's attach fields (`name`, `repo`,
`branch`, `build`, `test`, `toolchain`). After the graft's row, each one
the organism's record does not hold is attached by the host's `codebase
attach` (as `hale dna codebase attach` does, as the habitat), a
`codebase.attached` row; nothing is cloned until a leg works one. A graft
that goes on attaches none twice.

## Acceptance: the homelab case

`dna/tests/acceptance_homelab_test.hl` is phase 4's exit at fixture level.
After `hale habitat up` on an empty box, every step is a row of the head's
surface, called on its socket as the operator face calls its rows: a
machine of the LAN is added over SSH (`add_host`), authorized by its owner
and probed (`probe_host`); voice's own repository is trusted as its nursery
(`add_nursery`, `repo = "."`) and voice pinned from it; voice is grafted by
name onto the machine (`graft`) and applied there (`apply_allocations`)
from its own compose; a streamed `/v1/chat/completions` through it, at the
machine's name, answers as server-sent events from a local model on the
machine's network; the grafted organism's model map is pointed at voice's
endpoint on the machine (`set_model_map`) and its model call goes through
voice to that model (`probe_model`); and the reads (`index` with purpose
and pulse, `allocations`, `saplings`, `hosts`) list it placed and up. The fixture's
machine is an sshd container driving this box's docker daemon, named
`localhost` with its port; its voice is a checked-in minimal tree with
voice's shape (`dna/tests/onboarding/voice-sapling`) in a local git
nursery; its model is the cycle's canned OpenAI-compatible gateway. It
says, as `SKIPPED`, what it does not prove: voice's own programs, store and
broker (the real voice needs voice#44 and a network fetch); the Mac's
launchd node.
The real run adds them: a Linux box with docker as the host, Ollama on it
over the host's network, the Mac as a launchd voice node, the real voice
from the bundled nursery, all through the face.

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
- Only a test's habitat is taken down (`sweep`), and any other only by
  `hale habitat down --forget`, planned and confirmed (below).

## Destroying: plan, then confirm

A habitat act that destroys plans first, like terraform: one auditor
computes the plan, a list of items, and the destroy step executes exactly
those items one by one, in order, so what is printed is what is done and
nothing else decides what to remove (`dna/core/teardown.hl`). Each item
says what it is, its size or count, and whether it is recoverable (`no`,
or `yes` and how). Nothing is touched until the person types back the name
of what is destroyed; there is no `--yes` and no other way to confirm.
`--plan` prints the plan and exits. The confirmation is bound to the plan
the person saw: the act audits again before it executes, and when what it
would remove changed meanwhile (an allocation placed, a host changed, a
volume or slot appeared) it refuses (`plan_changed`), touching nothing, and
the person plans again. The rows carry the binding as the plan's `digest`
(each item's group, subject, action, host, arguments and recoverability,
never its sizes, which grow while the person reads). An item's arguments
name the resources it audited, each by its identity (every container,
volume and network of a compose project, sorted; a volume, a vault slot, a
path; a Forgejo account by its id and its repositories by theirs, each
repository deleted by id before the account is), and its execution removes exactly those: a
resource that appeared since is a changed plan, never one swept in by a
project's name. An allocation is planned where it is applied, as the
habitat recorded it (this machine unless it says otherwise); a placement
on another host not applied yet is audited there too. A destroy that fails
part way stops at the item that failed and says which items were done and
which were not, and leaves the rest; run again, it plans again from what
remains.

A host that does not answer over SSH is still removable. What the habitat
ran there is planned as `unreachable: left on <host>, cannot be removed
from here (not recoverable from the habitat)`, an allocation a line; on
confirmation the habitat's own side of it goes (its rows, its record of
where each allocation was applied, its slots and files here) and the
report names what was left there. The same holds in `down --forget` (which
keeps those lines as notes, like the authorized key lines) and in an
organism's removal.

Only what the habitat minted is removed from the vault. A slot a person
supplied (the forge token of a GitHub forge the Board allocated, a
`--key-vault` slot, a model key) is listed as kept, with that reason, even
when an allocation row names it.

**`hale habitat down --forget`** removes the habitat whole. Its plan,
grouped by what each item is:

- the habitat's record and memory: the record's row count, the organisms
  it grafted and the newest row's time, its memory's volume, and its own
  nerves and senses' store; its directory (its genome repository, the
  allocations' files and secrets files, its saplings, logs and state); and
  its body and face, stopped first;
- each grafted organism: its record (its memory's volume), its genome
  repository (the habitat's clone of a sapling is removed; a checkout
  outside the habitat is the person's, and kept), its forge repository
  (at the habitat's Forgejo, removed with it; GitHub's, kept), and its
  allocations by host (a heart's database and broker go with the memory
  and nerves servers that hold them);
- the compose projects and their volumes: the habitat's project and each
  organism's services project, their containers and networks, and any
  volume of theirs no other item names;
- the vault: the slots the habitat and its organisms hold, by name, never a
  value: the ones the habitat minted (memory's and the nerves' accounts,
  the OIDC clients, the record's key, Forgejo's administrator and each
  organism's token, the habitat's SSH key) are removed, and the ones a
  person supplied (a forge token, a `--key-vault` slot, the model keys) are
  listed as kept; a vault at `HALE_VAULT_ADDR` is provisioned out of band
  and kept;
- the habitat's Forgejo, its volume, and the accounts and repositories it
  holds;
- the hosts' entries, and the line each `ssh` host's
  `~/.ssh/authorized_keys` took, which the habitat cannot remove: it is
  noted, and the CLI removes it over the person's own login (above);
- what it runs on each `ssh` host, as the host answers over SSH: each
  allocation placed or applied there (its service stopped and removed, its
  volume deleted), the rest of the habitat's compose projects there, and
  the directories the renders were copied to (`~/.hale/habitat/<project>`).

The audit reads the records, so a memory of the habitat's project that is
stopped (a plain `down` stops them) is started for it and stopped again
when nothing is removed. The confirmation is the habitat's name typed back
at a terminal: when stdin is not one, `--forget` without `--plan` refuses
(exit 2) and says why; a name that is not the habitat's is refused (exit
1) and nothing is touched. Executed, what runs on the hosts goes first,
then this machine's projects, volumes, slots and files. The authorized key
lines are kept as notes beside the habitat's directory
(`forgotten-notes.jsonl`): the next plan shows each until a forget clears
it, so a second `--plan` after a forget shows those lines alone. On
`operator` backends the record is in the operator's Postgres, which the
habitat adopted: it is noted and kept.

**`hale habitat host remove <name>`** on a host with allocations plans the
same way: the plan (`Commands::plan_remove_host`, a read) is each
allocation placed or applied there and what stops or is deleted on that
host, the rest of the habitat's projects there, the directories copied
there, each allocation placed there unplaced in the record (its
`habitat.allocated` row with no host; `hale habitat place` places it
again), the host's entry marked removed, and the authorized key line,
noted. When the plan stops or deletes anything on the host, the removal
(`Commands::remove_host`) takes the host's name typed back as `confirm`
and refuses without it (`confirm_required`) or on a mismatch
(`confirm_mismatch`), touching nothing; a host that runs nothing of the
habitat's is removed without one. A host that does not answer over SSH
is planned as such: what runs there is not known and is left there. The
operator face's hardware screen shows the plan and a field for the name.

**`hale habitat organism remove <name> [--plan]`** removes one organism
the habitat grafted (by its name, or its key when two share a name), the
same way: the plan (`Commands::plan_remove_organism`, a read of the
Board's) is its record and memory, its genome repository (the habitat's
clone of a sapling is removed; a checkout of the person's is kept), its
Forgejo repository and account, its allocations by host (this machine's
volumes, another host's over SSH, an unanswering host's named and left),
its compose services and projects, its vault slots and its files here, and
its rows in the habitat's record: its `habitat.grafted` row and each
`habitat.allocated` row written again with `"state": "removed"`, after
which no reader lists it. `Commands::remove_organism` (`{name, confirm,
digest}`) executes it only with the organism's name typed back and the
plan's digest, refusing `confirm_required`, `confirm_mismatch` or
`plan_changed`; the CLI requires a terminal. The habitat itself is no
organism this removes (`not_removable`): `down --forget` is its removal.
The operator face's organisms screen offers `remove` on each organism, with
the plan and a field for its name.

A plain **`hale habitat down`** takes the habitat's compose project down
(`docker compose down`), leaving only its volumes.

## Verbs

`hale habitat [--head <url>] <verb>`; each verb prints its usage with
`--help`. Both `hale habitat` and `hale dna` accept `--head <url>` (and
`HALE_DNA_HEAD_URL`): precedence is `--head` flag, `HALE_DNA_HEAD_URL`
environment variable, the local habitat's recorded `head.url`, then its
`habitat.profile` row. The host verbs are clients of the habitat head's
Board commands (`Commands::add_host`, `::probe_host`, `::plan_remove_host`,
`::remove_host`, `spec/dna.md`): they call the head's HTTP surface (or its
socket), so the habitat's head must be up.

| Verb | Behaviour |
|---|---|
| `up --host <name> [--user <user>] [--port <n>] [--image <tag>] [--backends own\|operator] [--name <name>]` | Runs the habitat on another machine over SSH and keeps a pointer to it here (§ A habitat on another machine). Idempotent. |
| `up [--no-body] [--image <tag>] [--profile local] [--backends own\|operator] [--name <name>]` | Births the habitat the first time (named `--name`, `local` by default; above), then brings every recorded allocation up through the `compose` provider, migrating any whose recorded schema version is behind this toolchain's, records its placement (its host, its policy, its head and face; above) and the head's URL (`head.url` and `habitat.profile`), and starts the habitat's `body` and `face` services in its compose project (`docker compose up -d`, built from the embedded `Dockerfile` and `hale.pin` unless `--image <tag>` or `HALE_HABITAT_IMAGE` names an existing local image), waiting on their probes. `--no-body` births it, brings its backing allocations up and records its placement, and starts no `body` or `face`. Idempotent. `--profile` takes `local`; any other is refused (exit 2). |
| `image build [<checkout>]` | Builds the habitat toolchain image from `<checkout>` (`.` by default) using its `Dockerfile`, tagged `hale-habitat:<pin>`, and prints the tag on stdout. |
| `brief` | Prints the habitat's brief (below): the head's `Commands::brief`, asked over the head's surface as the person this account maps to. |
| `status` | The habitat, its name, its genome, body and face (its address), its head's MCP endpoint while the face runs, and each allocation: its organism, whether each role's service runs, the schema version memory and the nerves record against this toolchain's, and how a heart's declared migration last ran. |
| `down [--forget [--plan]]` | Takes the habitat's compose project down (`docker compose down`), stopping its `face`, its `body` and every service of its project and leaving only its volumes; `up` brings them back. `--forget` removes the habitat whole, planned and confirmed by its name typed back at a terminal; `--plan` prints the plan and exits (above). |
| `migrate [organism]` | Brings the organism's allocations up and migrates them: memory's schema and the nerves' stream move to this toolchain's, and the head's DSN is kept in the checkout's `.hale/dna/memory.env`. With `HALE_DNA_MEMORY_DSN_OWNER` or `HALE_DNA_NATS_URL_OWNER` set, that server is migrated instead. Idempotent; the organism defaults to the current directory. Runs the heart's declared migrations (below). |
| `sweep <root> \| --test-vaults [<root>]` | Takes down every compose project run from under `<root>`, a test's habitat, and confirms it gone by the project's label. Never the developer's habitat nor one outside `<root>`. Exit 1 names what survived. `--test-vaults` sweeps the vaults of processes that are gone. Nothing is done without docker. |
| `organism <name> dna <verb> [args…]` | `hale dna <verb>` in the grafted organism's own directory; through a pointer, on the habitat's machine over SSH (§ A habitat on another machine). |
| `graft <dir> [--attach] [--no-library] [--no-pack] [--profile local\|remote-body [--remote <url>] [--body <user@host>]] [--with <pack> [--set <param>=<value>]… [--as <who>]]` | Births an organism as the habitat's child, the one way an organism other than the habitat is born. See below. |
| `place <key>/<role> <host>` | Places an allocation on a host (`Commands::place_allocation`, above); nothing applies. |
| `apply [<host>] [--fresh]` | Applies the allocations where they are placed and prints the pulse (`Commands::apply_allocations`, above). |
| `host add <name> [--user <user>] [--port <n>]` | Adds a machine of the LAN as an `ssh` host (above): prints the host key first seen there and its fingerprint, what it offers, and the habitat's public key with the line its `~/.ssh/authorized_keys` takes. A host already added is refused (`host_exists`), and one where no SSH server answers is not added (`unreachable`). |
| `host probe <name>` | Probes the host again over SSH: a `pending-key` host whose key is now authorized is `ready`, with what it offers. A host that answers with another host key is refused (`host_key_changed`). |
| `organism remove <name> [--plan]` | Removes an organism the habitat grafted, planned first and confirmed by its name typed back at a terminal (above). |
| `host remove <name> [--plan]` | Removes the host, a `habitat.host` row in state `removed`, planned first: with allocations on it, the plan (each allocation and what stops or is deleted on that host) is executed only once the host's name is typed back at a terminal; `--plan` prints the plan and exits (above). `local` is not removed. |
| `graft <sapling> [--host <name>] [--own <part>[=<host>]]… [--reuse <part>]… [--plan] [--no-library] [--no-pack] [--set <param>=<value>]…` | Grafts a sapling the habitat pins, by name, its application placed on the host `--host` names and each body part filled as planned (below). |
| `nursery add <source> \| refresh \| list \| ratify <sapling>` | `add`: the Board trusts a nursery (`Commands::add_nursery`), then what it offers is read; `refresh`: every nursery read again; `list`: the nurseries and saplings (`Commands::nurseries`, `::saplings`); `ratify`: the filed revision pinned (`Commands::ratify_sapling`). |
| `forge <organism> forgejo\|github\|file\|none [--repo <owner/name>] [--key-vault <slot>] [--no-head]` | Allocates the organism's forge (above): the head's `Commands::allocate_forge` while it answers, the act itself with `--no-head`. |
| `handover [organism]` | Prints what the habitat hands the organism's heart, one `NAME=value` a line. |

The free-standing birth verbs are retired (`spec/dna.md`, Graft): each
exits 2 naming the graft that replaces it.

## The face

The habitat is served as every organism is: the standard face over its own
head (`spec/dna.md`, The habitat's face). `up` starts the `face` service
(`hale dna head <genome>`) beside `body` (`hale dna dev <genome> --no-iris`)
in the habitat's compose project, its state under `face-state` in the
habitat's directory. The habitat's address, and each organism's, is a block
of 8 loopback ports recorded in `dna.head.url` (the habitat's in its
genome's, an organism's in its own, assigned at its graft); the face is the
block's base and the head the next port (`http://127.0.0.1:<head_port>`,
recorded on the `habitat.profile` row and in `<habitat>/head.url`). The
face's index lists the organisms the record grafted, with their state and
the addresses of their faces and heads. A habitat started under
`HALE_DNA_TRUSTED_LOCAL=1` serves its face without the stub provider, as a
fixture's head does.

The genome carries a `ui/` (`dna/habitat/ui`), written at birth and tracked,
which the standard face serves under `/app/` as it serves any attached
project's: the operator face. A signed-in visitor lands on it: the render sets the
`face` service's `HALE_DNA_FACE_HOME` to `/app/`, so `/` redirects there,
the organization shell answers at `/organization` (linked from the
operator face's navigation), and `/app` redirects to `/app/`. Its first screen shows the sign-in edge and
the habitat's name, state, record revision and organisms, read by the
page from the head's `Commands::index` row through the generated ES-module
client committed beside it (`head-client.js`, from `dna/face/operator`). It
has no build step. A visitor with no session is redirected by the head's
gate before the page is served; a read refused for want of one is shown as
the crossing. The page's visual rules are CSS custom properties
(`operator.css`): near-black ground, one monospace, hairline rules, brass
only for a crossing, color only for state (up, degraded, down).

The page is one session with four screens, each a module of the same
directory, switched by the hash (`#habitat`, `#hardware`,
`#infrastructure`, `#organisms`), with no build step and no graph
renderer. Every call is the generated client's; the page speaks no other
HTTP. The head relays the surface's reads and the Board's acts for a
signed-in visitor: the standard face presents the visitor's session as the
call's bearer on any `Commands::` row, and the row keeps its own gate (a
person who does not hold the Board is refused as the row says).

- **Hardware** (`hardware.js`): the hosts (`Commands::hosts`) with state,
  offers, the host key fingerprint first seen there and the allocations
  placed on each (`Commands::allocations`). A form adds a host
  (`Commands::add_host`: name, user, port) and then shows the habitat's
  public key line for the machine's `authorized_keys`, with a copy button
  (the form and the line are crossings, so brass); a `pending-key` host
  offers `probe` (`Commands::probe_host`); every host offers `remove`,
  which asks the plan first (`Commands::plan_remove_host`) and, when it
  stops or deletes anything on the host, shows it with a field for the
  host's name, which `Commands::remove_host` checks (a crossing, so
  brass). A refusal is shown as the row answered it, and a removal's
  report item by item.
- **Infrastructure** (`infrastructure.js`): the allocations (role,
  instance, provider, host, the state the provider observes) and the policy
  (`Commands::policy`). A host picker per allocation calls
  `Commands::place_allocation`; `apply` calls `Commands::apply_allocations`
  and shows the pulse, what each provider observes of each allocation
  after it. A move of a memory that holds a record is refused by the row;
  the page shows the refusal's text with a confirm step that applies again
  with `fresh`.
- **Organisms** (`organisms.js`): the organisms the index lists (name,
  purpose, state, pulse) with their allocations, the saplings the habitat
  pins (`Commands::saplings`) and the nurseries it trusts
  (`Commands::nurseries`). A sapling has a host picker and `graft`
  (`Commands::graft`), which answers at once with the graft's id; the page
  follows the graft's log through `Commands::graft_log` (`{graft, from}`
  answering `{text, next, done, exit}`, the Board's like the graft) until it
  is done, then refreshes. `graft_log` reads only the log of a graft this
  head started, by its id, and refuses any other name or path. Each
  organism offers `remove`: its plan (`Commands::plan_remove_organism`)
  with a field for its name, which `Commands::remove_organism` checks with
  the plan's digest (a crossing, so brass).

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

The third screen, infrastructure (`infrastructure.js`, `#infrastructure`),
lists the allocations (`Commands::allocations`: role, instance, provider,
host and the state the provider observes) and the policy
(`Commands::policy`: the provider and profile each role is allocated by).
Each allocation has a host picker (this machine and the `ready` hosts) that
calls `Commands::place_allocation`; `apply` calls `Commands::apply_allocations`
and shows its pulse, what each provider observes of each allocation after it.
A move of a memory that holds a record is refused by the row; the page shows
the refusal's text with a confirm step that applies again with `fresh`.

The fourth screen, organisms (`organisms.js`, `#organisms`), lists the
organisms the index shows (name, purpose, state and pulse) with the
allocations placed for each, then the saplings the habitat pins
(`Commands::saplings`: name, purpose, revision, digest, nursery) and the
nurseries it trusts (`Commands::nurseries`). A sapling has a host picker and
a `graft` action (`Commands::graft`), which answers at once with the graft's
id; the page follows the graft's log through `Commands::graft_log` (`{graft,
from}` answering `{text, next, done, exit}`, the Board's like the graft; it
reads only the log of a graft this head started, by its id, and refuses any
other name or path) until it is done, then refreshes the lists.

## The solo path: no model

Every face screen works with the models catalog empty: no `model.category`
row, no seat, no key. With no model:

- **No verb requires one.** The operator face's screens read
  `Commands::index`, `::hosts`, `::allocations`, `::policy`, `::nurseries` and
  `::saplings`, none of which asks a model.
- **A person does the organism's work.** With no model (none mapped, none
  the catalog reaches; a canned or recorded one counts), an ask is a
  person's job, handed unassigned with no plan asked, and a Work a leg or
  the editor would perform is a person's case.
  `spec/dna.md` (Graft, and Workflow execution: human cases) has both rules.
- **The face does the person's part.** A person takes a handed Task
  (`Commands::take_task`) and reports it done with the work as text
  (`Commands::complete_task`, a `task.done` row in their name).
- **Nothing times out into a leg.**

The models screen's empty map is the normal state, not an error.

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

**A graft plans each body part before it makes anything.** The organism
plane (`memory`, `nerves`, `senses`) reuses the habitat's unless
`--own <part>[=<host>]` gives the organism a server of its own on that host
(on the `--host` machine without one). A reused `memory` is a tenant of the
habitat's own Postgres: the organism's record is a schema with roles of its
own there (as every record is: `dna_<id>`, `dna_<id>_spine`, …), on no
server of its own, and its heart's `database`, when the habitat provides
one, is a database of that server too. Removing the organism drops its
schema, its roles and that database there (`record-drop`, run inside the
habitat's memory by its superuser), and keeps the server; forgetting the
habitat removes them with its memory. A reused `nerves` or `senses` is a
server of its own on the habitat's machine: the nerves' configuration
carries one organization per NATS server (its users and their subjects are
the server's), so sharing the habitat's needs an account per organism,
which is not built. The application plane is the organism's own
software: its compose services, placed on the `--host` machine, and the
heart roles it needs (`database`, `broker`), which are the application's
own when its manifest says it brings them (`[dna] brings = ["database",
"broker"]`, a subset of its needs: its compose provides them) and are
otherwise provisioned by the habitat. `--reuse <part>` has the habitat
provide a heart role an application brings; `--own <part>` leaves it to
the application. `graft --plan` prints the plan, part by part (each
choice, where it runs, and the flag that changes it), and grafts nothing;
a graft at a terminal prints it and acts only once the person confirms.
A graft the head starts (the operator face's) asks no one and takes the
plan as its defaults give it. The choice for each heart role is kept
beside the organism's allocations (`parts.json`): `hale habitat migrate`
and an upgrade provision, and run the heart migration of, only the roles
the habitat provides. Identity is each organism's own sign-in stub for
unless the habitat's face has published its identity (below).

**One identity for the habitat and every organism it grafts.** The
habitat's face starts its sign-in provider with `HALE_DNA_IDENTITY=publish`:
its signing key is kept in the face's state, made once, so it outlives a
restart, and the provider is published at `<habitat>/identity.env` (its
issuer, the key's public half, the client `dna-local`, and where it
answers). A graft points the child at it (`dna.oidc.issuer` in the child's
configuration, which its head, its heart's handover and the host's code
read) and registers the heart's service client in
`<habitat>/identity.services`, which the provider reads at each request.
An organism's face launch under that habitat starts no provider of its
own: it signs in through the habitat's, so a person has one account for
every organism, and what they may do in each is the positions they hold
in its record. The plan says `identity reuse the habitat's sign-in`.

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
| `habitat.profile local` | at birth | `profile`, `backends`, `project`, `name`, `face`, `head` |
| `habitat.grafted <child>` | at a graft | `child`, `parent`, `needs`, `backends`, `name` (the directory), `face` and `head` (the organism's addresses, recorded under the habitat's declared address, `HALE_DNA_HABITAT_ADDRESS`: `127.0.0.1` locally, the host's name on another machine, so the operator face's links reach them from the admin machine; `hale dna head` under such a habitat, as `hale habitat organism <name> dna head .` runs it there, binds the habitat's listen address and answers under its declared address, as the habitat's own face does, and keeps its state in a directory of the organism's own, `organisms/<record>` under the state directory every run there shares), `purpose` (its declared purpose), and for a sapling's graft `sapling`, `revision`, `digest` and `nursery` |
| `habitat.applied <key>/<role>` | at each apply of an allocation | `host`, `provider`, `ok`, `why`, `state` (the pulse after), `fresh` |
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

Every compose project the habitat applies on an `ssh` host (an organism's
own services placed there, an allocation placed there) is also handed two
facts of the host in its environment, whatever it runs; the habitat knows
none of the project's own variables, and a project maps these in its
compose file. On this machine neither is set.

| Variable | Value |
|---|---|
| `HALE_DNA_HOST_NAME` | the host's name, as it was added (`hale habitat host add <name>`) and as the LAN's DNS resolves it |
| `HALE_DNA_HOST_LISTEN` | the host's listen address: the first IPv4 address its name resolves to on the habitat's machine, through the toolchain's own resolver (`hale habitat resolve <name>`), which the LAN reaches it at; an apply on a host whose name resolves to none is refused, saying so |

The manifest is read once, by graft; every reader after it reads the
roles the habitat allocated, as `allocation.json` keeps them.
