# The habitat

This document specifies the habitat as shipped (GH #1653). It describes
the `local` profile, the only one built; the hosted profile is not. The
organism it serves is specified in `spec/dna.md`; the design it came from
is `docs/src/habitat.md`.

## What the habitat is

The habitat is a DNA organism the compiler ships, as it ships the
foundational law and the nursery packs. Its genome is fixed: its positions
are the nursery's `habitat` pack (four seats under the Board, every one
held by the person who ran `up`), its law the `standard` pack and its
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
| `offers(host)` | the host as the provider produces it: its name, the provider, what it offers (`compose`, `systemd-user`, `nvidia-gpu`), or why it cannot be reached |
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
(`hale dna head <genome>`), and the body of each organism grafted from a
sapling (`body-<key>`, § A sapling's body), run as services of the
habitat's compose project,
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
compose -p <project> --project-directory <dir> -f <file>… up -d --build --wait
[<service>…]`, with `COMPOSE_IGNORE_ORPHANS=1`: every allocation the habitat
brings up, the habitat's own and each tenant's, is brought up this way.
Each allocation service has a memory limit (compose's `mem_limit`): a
Postgres 512 MiB, a NATS 128 MiB, a senses' store 512 MiB, the forge
384 MiB, several times what each holds at rest, so a runaway is a restart
the operator sees rather than a small machine swapping.
`--build` builds a service whose image comes from the render's own context
(an organism's application) from the revision being applied, so an image an
earlier revision built under the same tag is never what runs; docker's
cache keeps an unchanged context cheap, and a service that only names an
image builds nothing. It
observes a service by its container's state (`running` is up, `running` and
unhealthy or `restarting` is degraded, anything else or no container is
down; a one-shot that exited 0 is finished, never down) and the head and
the face by whether their address accepts a connection. Of a project's
containers it names each that is not running with the code it last exited
with and how often docker restarted it (`3 of 4 running; not running: api
(exited 139)`, `api (restarting, exited 139, 7 restarts)`), and each that
runs again after docker restarted it (`restarted: api (2 restarts)`, from
`docker inspect`'s `RestartCount`); one that came back only seconds ago
(`up 4 seconds`) leaves the allocation degraded, since between two crashes
a crash loop is running. The `ssh` provider observes the same way, over
SSH. A provider in memory (`MemProvider`) stands in for it in tests.

**Kept running.** The habitat owns the lifecycle of what it applies, so a
crash is a restart docker makes, not a service down until someone looks.
Every long-running service it runs has the restart policy
`unless-stopped`: the memory, nerves and senses' store of an allocation
and the forge in the compose files the habitat writes, and an organism's
own services through the habitat's override (below), unless the
organism's compose files state a policy of their own (`restart:`, or
`deploy.restart_policy`), which is left as it is. `unless-stopped` rather
than `on-failure`: a service that exits 0 when it was not meant to stop is
brought back too, and so is every service when the host's docker restarts
(a reboot), where `on-failure` leaves them down; a service the habitat
stops or withdraws (`hale habitat down`, a move, a removal) stays stopped.
A **one-shot** — a service another waits on to finish (`depends_on` with
`condition: service_completed_successfully`, a migration) — gets no
policy, since running it again after it finished would be wrong; a one-shot
nothing waits on is read as long-running, so its compose file says
`restart: "no"`. Docker backs off between restarts, and the observation
above names a crash loop, so a policy never hides a failing service. The
habitat's `body` and `face`, and its organisms' bodies, keep their
`on-failure`: their clean exit is a drain the habitat asked for.

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
that runs a habitat of its own points at none. A second `up --host` with the
pointer's name upgrades that habitat in place; its host may be named another
way (a bare `rack-a` for a recorded `rack-a.example.net`, or any name resolving
to an address the recorded one has), and the pointer keeps the spelling it
recorded. Another name or another machine is refused, with the `up` that
reaches the habitat this machine points at.

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
takes only a directory under `<habitat>/grafted/` that exists there. A
nursery that is a directory on the admin machine is staged on it and
what was staged shipped there the same way (§ Nurseries and saplings, A
directory nursery). The organism's own verbs (`hale dna review`,
`task create`, `work run`, `show`, …) run in its directory there through
`hale habitat organism <name> dna <verb> [args…]`, at the person's terminal
when there is one; locally the same verb runs them in the organism's
directory here. `migrate` and `handover`, which act on a genome on the
admin machine and have no route through the head, are refused while the
pointer names another machine.
`up --host` again finds the habitat there by its record's identity (one
whose identity differs is refused, nothing changed), runs the same
idempotent `up`, which changes nothing, and writes the pointer again.
`up --host` on a machine with no pointer, to a host where a habitat
already runs, is refused with nothing changed, naming where the pointer
would be: pointing a second directory at a running habitat (a shell whose
`XDG_DATA_HOME` names another habitat's, a pointer lost) is a choice,
`--attach`, which points this machine at it and says so (`attached`);
`--attach` where none runs is refused too. `up` and `status` name the
directory they use, and what named it when it is not the default
(`XDG_DATA_HOME` or `HALE_DNA_HABITAT`): `up --host` ends with a `pointer`
line.
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
does, `nvidia-gpu` when `nvidia-smi -L` lists a GPU there, and its OS and
architecture. A host that does not accept the
habitat's key yet is `pending-key`, and `hale habitat host probe <name>`
moves it to `ready` once it does. The provider applies a render by copying
its files to `~/.hale/habitat/<project>/<render directory>` on the host
and running `docker compose -p <project> -f <file>… up -d --build --wait` there;
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
an organism's body runs on the habitat's machine (§ A sapling's body), and
an allocation on the `operator`'s server is never placed.

**Apply** is the Board's command `Commands::apply_allocations` (`{host,
fresh}`, `hale habitat apply [<host>] [--fresh]`), over every allocation
or those placed on or leaving one host. An apply takes minutes (an
organism's images are pulled or built on its host) and the head serves its
commands one at a time, so the command starts it in a process of its own
(`hale habitat apply --no-head` on the head's own record, the host verb
`habitat-allocations-apply`)
and answers at once: `{ok, code: "started", apply, log}`, the apply's id and
its log (`<habitat>/applies/<id>.log`, its exit status beside it in
`<log>.exit` and what it came to in `<log>.reply`). One apply runs at a
time: while one runs the command starts none and answers `code:
"applying"` with the running one's id and log, to follow.
`Commands::apply_log` (`{apply, from}`, the Board's like the apply) pages
the log: `{ok, code, why, text, next, done, exit, outcome}`, `outcome` once
it is `done` the apply's reply as a JSON object (`{ok, code, why, restart,
applied}`), and `{}` when it ended without one (its process was stopped). It
reads only the log of an apply this head started, by its id, and refuses
any other name or path. `hale habitat apply` is a client of both: it says
what the apply is applying as it goes and prints the pulse when it ends,
exiting 0 when every allocation applied; through a head older than its
toolchain, which applies inside its request, it waits until that head
answers a read again and prints what each provider observes then. What an
apply does:

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
places the new organism on that host before anything of it is brought up;
`--host local` places it on the habitat's own machine, through the
`compose` provider, its services in a compose project of their own there.
A graft that names no host places its services exactly as `--host local`
does where the graft runs, and says so (`placed  its services (<key>/services)
on <machine> (no `--host`: the habitat's own machine)`); its organism-plane
parts stay where the plan puts them (the habitat's, or `--own <part>` on the
habitat's machine). Through a pointer the graft runs on the habitat's
machine, so with no `--host` its services go there, as `--host <the
habitat's machine>` places them, never on the machine the person types on
(which `--host local` names there). A graft never leaves an organism's
services unplaced.
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
the organism's catalog would have chosen (a catalog with no backend of its
own among them, as a graft with no model key writes it), and a key slot
the vault lacks is refused there, never answered by another backend. The
habitat's models fill a tenant's map without a person: an organism the
habitat grafted that declares `[dna.provides.models]` (voice) is every
organism's model backend, each keyed by a member of its drawn family
(`spec/dna.md`, The habitat's models; `hale habitat models`). A sapling's graft is the head's Board command
`Commands::graft` (`{sapling, host, library}`): it starts the act, which
takes minutes, and answers at once with its log (`<habitat>/grafts/<name>.log`,
its exit status beside it in `<log>.exit`); the end of the graft is its
`habitat.grafted` row. `hale habitat graft <sapling>` is a client of it while
the head answers, following the log to the exit, and the act itself
otherwise or with `--no-head` (what the head runs, given the `--host` the
command named, `local` included: a graft that names no host places its
services on the habitat's own machine, as `local` does).

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

**The chart's personas** are Forgejo users of their own, so the habitat's
forge shows the parts of each organism discussing and cooperating rather
than one account talking to itself. Every organism with a repository at
the habitat's Forgejo has one persona per position its chart states (a
proposed position has none until the Board states it), one for its Board
(`position:board`, stated or not), and one per person who holds a position
there. A position's and the Board's user is the organism's:
`<organism user>-<name>`, a `/` of the position's name as `_` (Forgejo
takes no doubled dash), and its token is the vault slot
`forgejo-<key>-<slug>`, the slug the name with each `/` as `--`
(`forgejo-<key>-api--dev`, `forgejo-<key>-board`). A name longer than
Forgejo's 39 characters, or one that is not lowercase letters, digits and
single dashes, is cut and tidied and carries 8 hex digits of a hash of
the organism's user and the position's id, so it is the same at every
apply and never another position's. A person's user is the habitat's,
one however many organisms the person is a member of: the member name
(`holds:position:<p>|<who>`'s holder), or, for a name that is not plain,
too long, or one Forgejo or the habitat keeps (`hale-admin`, `admin`, …),
the name tidied with a hash; its token is `forgejo-person-<user>`. Each
persona's user writes to its organism's repository (a person's to every
repository of an organism that has them), has an avatar drawn from its
name (a 96×96 identicon, uploaded as the persona, kept as data of the
persona's so a later avatar can be chosen per hat) and the name Forgejo
shows (`dev of shop`, `the Board of shop`, the member's name). A
persona's user is marked by its email, `<user>@<key>.persona.habitat.invalid`
or `<user>@person.habitat.invalid`; only a user so marked is ever removed,
and a user of the name that is not so marked is said as a conflict and
left alone. No token is printed: it goes from Forgejo into the vault.

`hale habitat graft` reconciles the organism it grafts once its own user
is made, and every `hale habitat apply` reconciles every organism (an
allocation `<key>/personas` among what it applied): a persona missing is
made and granted the repository; one whose position retired or closed is
removed with its slot; a person's user and slot go once no organism's
chart has the person; an organism no longer grafted loses its personas.
A persona whose user exists is left alone, its token minted again only
when its slot is missing, so a second apply changes nothing. An
organism whose chart cannot be read keeps everything it has, and no
person is removed while any chart is unread. A chart that cannot be read
(its memory just moved, recreated or not answering) never makes the apply
or the graft refuse: its organism's personas are `skipped (chart not
read: …)` and left as they are, and the next apply reconciles them; nor
does a Forgejo that does not answer. The chart is read with `hale dna
chart` (the core's `org_chart`: stated positions, holds neither refused
nor of a retired person, and a `person <name>` line per person holding a
position); the plan is made by
the toolchain (`crates/hale-cli/src/dna/personas.rs`) and carried out by
`habitat-forge-personas`. Each organism's personas as the last reconcile
left them are kept beside its allocations (`allocations/<key>/personas`),
and `hale habitat status` shows their count under the organism.

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
leg's branch to a repository at the allocated Forgejo, its token handed to
git through a credential helper reading a private file. The guard refuses
a `Hands` whose forge names a kind.

**A leg acts as the persona of the position its work is for.** Forge auth
is the legs runner's, never a harness's or voice's: a leg handed work (a
branch of commits in a worktree, a person's or a harness's alike) puts the
position the Work is for on its hands (`wear`: the hat's `position.id`,
else the leg's `--as`), and at the allocated Forgejo the git hand pushes
the branch as that position's persona (its user the credential helper's
username, its token the persona's slot), the forge hand opens the review
with the persona's token, so the pull request's author is the persona's
user, and the commits the leg itself adds there (one of what the work
left uncommitted, a fixup) are authored as the persona's user and email.
The commits a person or a harness made stay on the branch as they were
made, their shas and authors: never squashed or rewritten (whether to
squash is decided at merge, on the forge). The persona is the one the
organism's ledger (`allocations/<key>/personas`) names for the position
— the reconcile's names, never derived again by the leg. A position the
ledger does not name yet (personas not reconciled, an older habitat, a
position the chart does not state), or a persona whose slot holds no
token, falls back to the organism's own user and slot, as before, and the
leg says so once on its output (`hale dna work: note: …`), never as an
error. With no position, a GitHub allocation or none, nothing changes:
verdicts are read with the organism's own token in every case.

**Publishing to GitHub.** Where the forge is the habitat's Forgejo, GitHub
is a sink the owner feeds in batches: `hale habitat forge publish
<organism> [--branch <name>] [--dry-run]` carries the main of the
organism's repository at the Forgejo to GitHub as **one pull request per
publish**. Where it goes is a `habitat.publish <id>` row (`{organism,
provider, repo, key_vault}`), recorded by `hale habitat forge <organism>
github --repo <owner/name> [--key-vault <slot>] --publish` as the act
itself, never through the head, and beside the forge allocation, which it
leaves as it is (an organism has one forge; a GitHub forge would take the
Forgejo's place in the gate); `none --publish` stops it. Its token is the
vault slot the row names, `forge-token` by default as a GitHub forge's;
the Forgejo's is the organism's own, `forgejo-<id>`. A publish fetches the
internal main, pushes it to GitHub as the branch `sync/<YYYY-MM-DD>` (the
UTC date; `--branch` names another, never `main`), and opens one pull
request from it against GitHub's `main`, titled `Publish <date>`, whose
body lists the internal pull requests merged since the last publish by
title and internal number (`internal PR <n>`: no `#<n>`, which GitHub links
to its own, and no link into the private forge). A title is made inert on
its way: put on one line, with a zero-width joiner (U+200D) after every `#`
and `@`, so a `#12` in it links no issue of GitHub's and an `@name`
mentions no one, while it reads as written. "Since the last publish"
is read from git, not kept: a merged internal pull request is listed when
its merge commit is on the internal main and not on GitHub's, so the first
publish lists everything since GitHub's main. Run again the same day, the
branch moves forward and its open pull request is updated, its title and
body rewritten; no second one is opened. `--dry-run` prints what it would
push (and any merge it would make), whether it would open or update, and
the pull request's title and body, and pushes and opens nothing (it reads
both forges). **Nothing is forced, and the internal main is never
touched.** When GitHub's `main` holds the internal main already (a merge
made there of a publish's pull request, and nothing new since), there is
nothing to publish. When it has commits the internal main lacks (GitHub's
merge queue merges with a merge commit), the branch pushed is a merge of
the internal main (first parent) and GitHub's `main`, made in the
publish's scratch repository; so is a merge with the branch an earlier
publish the same day left at GitHub, which the new one then
fast-forwards. A merge is authored and committed as the owner the GitHub
token belongs to (GitHub's `/user`: its name, and its public email or its
`<id>+<login>@users.noreply.github.com` address), and the pull request's
body says the branch merges the two. A merge that conflicts stops the
publish, naming the files, and nothing is pushed: the conflict is resolved
in the internal forge. It also refuses, before anything is fetched, an
organism with no Forgejo allocation, one with no `habitat.publish` row
naming GitHub (saying how to record one), a vault holding no GitHub token
in the row's slot or no token in the organism's Forgejo slot, and a vault
served over HTTP (`HALE_VAULT_ADDR`). GitHub's `main` must exist. **A
habitat on another machine publishes there**: with a pointer (`up
--host`), both `forge publish` and `forge … --publish` run on the
habitat's machine in its toolchain image (`remote::run_there`, as `nursery
refresh` and `models` do), where its record, its vault and its Forgejo
are; the output streams back. On a habitat machine that is a Mac, docker
runs in a VM and `--network host` is the VM's network, not the Mac's, so
the publish's run is given `HALE_DNA_LOOPBACK_HOST=host.docker.internal`
(and that name mapped to the host's gateway), and it dials the Forgejo's
recorded `127.0.0.1:<port>` there, as the habitat's `body` and `face`
reach its loopback (`dna::reach_url`). The tokens go to git
through a credential helper reading git's own environment and to curl,
which speaks both forges' APIs, in a config read from its stdin: never on a
command line.

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
organism already grafted stays at the revision it was grafted at until
`hale habitat organism upgrade` moves it (below).

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
`--set` values, each to the packs that take it; `founder` is the person
grafting unless either names another, so a pack's `? fill … {founder}`
lines seat them, the Board's seat first, and the child settles its Reviews
with nobody else asked. A sapling that declares packs declares its chart:
its birth proposes the Board's seat and no other hole, since the pack's
positions are the chart (`graph-ingest --chart-declared`). **A sapling's packaging is
ratified by its graft**: the person who grafts it chose it, so the graft
then decides every pending group in their name (`hale dna review all
approve --no-wait`, its verdicts executed when the child's body runs, which
the habitat keeps: § A sapling's body) and
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

**`hale habitat organism upgrade <organism> [--plan]`** moves an organism
grafted from a sapling to the revision the habitat pins for it now, the
one its Board ratified, keeping its record, its data and its allocations
(a removal and a new graft would drop them):

- **Where it stands.** The revision its `habitat.grafted` row names, or the
  latest `habitat.organism.upgraded` row's `to`. It is refused when that is
  the pinned tree already (the digests agree), saying so; a newer revision
  filed and not ratified is named there, with the `nursery ratify` that pins
  it. An organism grafted from a directory, or whose sapling the habitat
  pins no longer, is refused by name.
- **The revision fetched and checked**, as a graft fetches and checks it:
  into the sapling's store, `<habitat>/saplings/<name>`, which is the
  organism's own directory; its digest computed from the revision's tree
  and checked against the pin. A mismatch refuses and nothing moves. A tree
  the toolchain carries is made a repository beside the store, checked as it
  is made, and fetched from there.
- **What it needs** at the new revision (`[dna] needs`) is what its graft
  allocated: a revision that needs a role more is refused, since an upgrade
  allocates nothing new.
- **The tree moved.** The store's checkout is detached at the revision's
  commit. What its graft wrote into the tracked tree is carried: its
  manifest's `[dna] record` pin and the tables the graft added (`[claims]`
  and `[environments.*]`, unless the new revision declares its own
  environments), the lines it added to `.gitignore`, and `hale.lock`, which
  the toolchain re-pins. Any other change to a tracked file is carried only
  when it applies cleanly at the new revision; one that does not, a manifest
  changed beyond its graft's tables, or a file held untracked (its
  organization, `vendor/dna`, its state) that the new revision tracks with
  other bytes refuses the upgrade before anything moves. A move that fails
  part way is put back. The store's kept graft (`<name>.grafted.json`) names
  the new revision, so a graft that goes on there goes on at it.
- **What it runs, re-applied**: `vendor/dna` re-materialized when the graft
  wrote one, the heart's declared migrations run (§ Opt-in heart
  migrations), and its services allocation, when its graft placed one,
  recorded again with what its render ships at the new revision and applied
  through its provider (`Commands::apply_allocations`), which builds its
  images from the new tree. A step that fails leaves the tree moved and the
  upgrade unrecorded, and the upgrade run again finishes it.
- **Its body**, which the habitat keeps (below), made again on the tree as
  it is now (`docker compose up -d --force-recreate`): it drains, and starts
  on the new revision.
- **The row**: `habitat.organism.upgraded <key>`, `{organism, child,
  sapling, from, from_digest, to, digest, commit}`, written into the
  habitat's record as the graft's row is.
- **Its host commands**: each placement of the organism that holds
  (`organism place`, § Host commands: an organism's own, on a host) runs its `install` again at the new
  revision, as `place` runs it, over the person's SSH logins. At a terminal
  each one runs (sudo asks there for a command with `root = true`); off a
  terminal only one known not to need root does, and the rest are printed
  as the `hale habitat organism place …` lines left to run.

`--plan` prints each step (from and to, the tree, the migrations, the
services re-applied, the row, the placements re-run and which need a
terminal) and moves nothing; it fetches the revision into the store's
objects to read it. Through a pointer (§ A habitat on another machine)
everything but the placements runs on the habitat's machine, in its
toolchain image (`--no-placements`), and the placements from this one,
where the person's SSH logins are.

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

**A directory nursery** is read where it is, at its committed `HEAD`
(its own sapling pinned at that commit by that tree's digest; the
working tree's uncommitted changes and untracked files never reach a
sapling), and staged under the habitat's directory of the machine that
read it: its index and packs, as the directory holds them, as
`nurseries/<key>`, and a mirror (`git clone --mirror`) of each sapling
repository that is a directory on that machine, its own among them, as
`nurseries/repos/<key>`. Each key is the first eight bytes of the sha256
of the source or repository path as named, in hex, so it is the same on
every machine. A nursery read again where its directory is not reads the
staged copy, its own sapling pinned at the mirror's `HEAD`, and a graft
or an `organism upgrade` whose sapling's repository is not on the
machine fetches from the mirror. **A directory nursery works for a
habitat on another machine** (a pointer, `up --host`): `hale habitat
nursery add <dir>` stages it on the admin machine, then ships what was
staged to the habitat's machine, packed with `tar` and unpacked over the
pointer's own SSH login (its user and port) into the habitat's directory
there (`<home>/.local/share/hale/habitat/local` of that login, the
`HALE_DNA_HABITAT` every run there is given) under the same keys, each
copy replacing the one there whole; only then does the Board trust it,
so the head reads it at once. Adding a directory the Board trusts
already reads the shipped copy there. `hale habitat nursery refresh`
through a pointer stages and ships again every trusted directory nursery
that is a directory on the admin machine, then reads the nurseries
there, so a commit made since is filed for the Board as a newer
revision, and the graft or upgrade after its ratification fetches it
from the new mirror. The copy is written as the login's user, the uid
every run there in the toolchain image runs as, so it is theirs to read.
A nursery that cannot be read on the admin machine is never shipped:
`add` refuses it and nothing is trusted; `refresh` names it, ships the
rest, reads the nurseries there and fails. A git nursery, and a sapling
at a git URL, are fetched where the habitat runs, as on one machine.

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

### A sapling's body

**The habitat keeps the body of every organism it grafted from a sapling**:
the organism's `hale dna dev <root> --no-iris`, which runs its
organization (its positions, its legs, the Board's verdicts executed), as
the habitat's own body runs the habitat's. A sapling lives in the habitat's
store, where no person runs it; a directory grafted from a checkout is the
person's, and its body is theirs (`hale dna dev` there). The body is an
allocation, `habitat.allocated <key>/body` (`{organism, role: "body",
service: "body-<key>", project, provider: "compose", host: "local"}`),
recorded by the graft, and by the next `up` or apply on the habitat's
machine for an organism grafted before bodies were kept.

It is a service of the habitat's compose project, `body-<key>`, beside the
habitat's `body`: the same image, user, network, mounts (the habitat
directory, the vault, the docker socket, and the organism's directory when
it is not under them), the same environment (`HALE_DNA_COMPOSE=off`: its
application's compose services are its `services` allocation, applied
apart) and the same restart policy (`on-failure`, its clean exit a drain)
and drain window, its working directory the organism's, labelled
`hale.role: body` with the organism's name. Its readings are not served
(`HALE_DNA_SENSES_PORT=0`): on the host's network their fixed ports are the
habitat body's. On a project network (macOS) it is given this machine's
hostname, so the lease it takes names the machine `hale dna status` asks
from. `dev` rather than `run`: `dev` derives the organism's memory, nerves
and handover from its allocations at each start, so a body that restarts
after a migration or a move reaches them where they are now; `run` takes
them from its environment, which the compose file would have to carry.

What the service takes of this machine is what `up` gave the habitat's own
body, which only `up` knows: `up` keeps it beside the habitat
(`body-shape.json`), and the organism's compose file
(`allocations/<key>/body.yaml`) is written from it. It is brought up
(`docker compose up -d --no-deps body-<key>`, not waited on: a body comes up
while it builds its genome):

- at the end of the graft, which says `body    its body (<key>/body) on
  <machine>`;
- at every `up`, after the habitat's own body and face, made again on this
  toolchain's image when that changed (`body    <key>/body of <name> up …`);
- at every apply on the habitat's machine (`hale habitat apply`, `apply
  local` or no host), after the allocations, listed in the apply's reply as
  `<key>/body`;
- made again at `organism upgrade`.

A habitat that runs no body of its own (`up --no-body`) keeps no shape: its
organisms' bodies are neither recorded nor started, and a graft says so.
`down` stops the organisms' bodies, drained, after the face and with the
habitat's own body, before their backends; `up` brings them back.
`organism remove` stops and removes it with the organism's other services
on this machine, before its memory is dropped, and `down --forget` stops it
first and removes it with the habitat's project. `status` shows it under
the organism's allocation (`body    body-<key> up (…)`, or `down`), and the
head observes it as it observes any compose service.

**Its face and head run beside it.** The graft records the organism's head
block (`face` and `head` of its `habitat.grafted` row, the block's first two
ports), and the same compose file holds a second service, `face-<key>`,
allocated as `habitat.allocated <key>/face` (`role: "body"`, `service:
"face-<key>"`): the habitat face's image, user, network and mounts, the
habitat face's environment less its own state and its published identity (it
signs in through the habitat's), its state in `allocations/<key>/face-state`
(`HALE_DNA_HEAD_STATE`), running `hale dna head <its directory>`, restarted
on failure. On a project network (macOS) its head block is published on the
habitat's listen address as the habitat face's is, Grafana's port left out;
on the host's network it binds its ports itself. It comes up, stops and is
removed with the body (`docker compose up -d --no-deps body-<key>
face-<key>`; a removal takes every `role: "body"` service of the organism),
so the `face` and `head` the habitat's index lists for the organism answer.
A shape kept by an `up` from before faces were kept carries no face
environment, and its organisms get a face at the next `up`.

**The body stays on the habitat's machine**, wherever the organism's
services are placed: it runs the habitat's image with the habitat's
directory and vault mounted, which another host does not hold, and it
reaches the organism's memory and nerves where their placement records
them. `place <key>/body <host>` is refused (`not_placeable`); moving a body
is not built.

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
- An allocation that does not come up is said in its own words. When
  compose cannot bring an allocation up, or its server starts and stops
  again (a full disk stops Postgres's `initdb`; compose's answer and an
  `exec` refused while the container restarts say nothing of it), the
  failure carries, for each container of the services it waited on that is
  restarting, exited with a failure, dead, never started or unhealthy, its
  state and the last lines it wrote, an older error line first when the
  retries' lines bury it (`dna::compose_trouble`): memory, nerves, the
  senses' store, the Forgejo, a tenant's allocations and the habitat's
  `body` and `face` alike. It names no knob that would not help.
- `up` after an `up` that failed converges. A birth is resumed from the
  steps it recorded, and memory decides whether its record was born: an
  identity whose record never reached memory (its memory never came up,
  or never had a volume) is drawn again, and what was made for it goes
  first: its allocation (services and volumes), the head's DSN derived
  from its memory (`.hale/dna/memory.env`) and the vault slots minted for
  it alone (memory's roles and superuser, the nerves' accounts, the
  record's key, its SSH key and Forgejo token; a slot of the habitat's own
  stays). The `up` after the cause is gone (a disk freed) births the
  habitat whole, and the next changes nothing.
- The genome is committed as it is born, since the head and the face
  read it from git (its organization from `HEAD`'s tree): the toolchain's
  files in one commit, then, once its organization is written, the seed
  under `dna/org`, `hale.toml` (the record's pin), `hale.lock` and
  `.gitignore` in another, each by name, so nothing the ignore file keeps
  out (`vendor/`, `.hale/`) and nothing else in the directory is ever
  committed. A birth resumed commits what it wrote; an `up` on a habitat
  born whose seed is not committed commits it, and otherwise commits
  nothing.

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
  the OIDC clients, the record's key, Forgejo's administrator, each
  organism's token and its chart's personas' tokens, each person's token
  there, the habitat's SSH key) are removed, every account the
  habitat's and each organism's organization holds on the nerves among
  them (`nats-<org>-<role>`, and `nats-<org>-app-<name>` for every name an
  application was attached by, of every organism the record says it ever
  grafted), and the ones a
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
  the directories the renders were copied to (`~/.hale/habitat/<project>`,
  and `~/.hale/habitat` itself when that leaves it empty);
- each service placed on a host (§ Host commands), taken off first of all
  by its own `uninstall` over the person's login, while the head and the
  organism it answers to are still there, then what placing installed on
  each host that no placement left there uses (the organism's checkout,
  the toolchain directory) and the habitat's directories there left
  empty. One that cannot be taken off is said with its `unplace` and the
  forget goes on. Through a pointer (`up --host`) the person's machine
  takes them off over its own login before the forget there runs, asking
  for the habitat's name for that step first, and the forget there notes
  any still recorded.

The audit reads the records, so a memory of the habitat's project that is
stopped (a plain `down` stops them) is started for it and stopped again
when nothing is removed. A habitat whose record cannot be read (a first
`up` whose memory never came up, so its record was never born; a memory
that does not run) is still forgotten: its plan is made from what this
machine holds, its directory, the compose projects, containers and volumes
labelled with its project, and the vault slots named for its record's
identity (`habitat-forget-plan --unrecorded`), and its first item says so,
naming why the record could not be read and that an organism, a host or a
placement only the record names is not in it. The confirmation is the
same. The confirmation is the habitat's name typed back
at a terminal: when stdin is not one, `--forget` without `--plan` refuses
(exit 2) and says why; a name that is not the habitat's is refused (exit
1) and nothing is touched. Executed, what runs on the hosts goes first,
then this machine's projects, volumes, slots and files. The authorized key
lines are kept as notes beside the habitat's directory
(`forgotten-notes.jsonl`): the next plan shows each until a forget clears
it, so a second `--plan` after a forget shows those lines alone. A note
lasts only while it is undone: a forget that keeps none removes the file,
the habitat's directory and the `habitat` directory holding it when they
are left empty, and a forget through a pointer, once the person's machine
has taken a host's key line (or the habitat's account) off over its own
login, drops that host's note on the habitat's machine and removes the
file with the last. On
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
services placed on the host (§ Host commands) head the CLI's plan: each
one's `uninstall`, then what placing installed there; a host with one is
removed only with its name typed back, and the CLI takes them off (over
the person's login) before the head removes the host, and removes the
habitat's directories there left empty after. The
operator face's hardware screen shows the plan and a field for the name.

**`hale habitat organism remove <name> [--plan]`** removes one organism
the habitat grafted (by its name, or its key when two share a name), the
same way: the plan (`Commands::plan_remove_organism`, a read of the
Board's) is its record and memory, its genome repository (the habitat's
clone of a sapling is removed; a checkout of the person's is kept), its
Forgejo repository and account (and its chart's personas there, each by
the email that marks it, with each person's user and slot that no other
organism's chart has, by the personas each organism's last reconcile
left), its allocations by host (this machine's
volumes, another host's over SSH, an unanswering host's named and left),
its compose services and projects, its vault slots and its files here, and
its rows in the habitat's record: its `habitat.grafted` row and each
`habitat.allocated` row written again with `"state": "removed"`, after
which no reader lists it. `Commands::remove_organism` (`{name, confirm,
digest}`) executes it only with the organism's name typed back and the
plan's digest, refusing `confirm_required`, `confirm_mismatch` or
`plan_changed`; the CLI requires a terminal. The vault slots it removes
include every one of its organization's accounts on the nerves
(`nats-<org>-…`, its applications' among them, which no row names), its
slot `model-HABITAT_<KEY>`, and its key at the habitat's models provider
(its member of the provider's family, an item of its own whose execution
also restages the provider's reader vault where its services run). Its
services placed on the hosts are planned by the CLI beside the head's
plan and taken off (each one's `uninstall`, then what placing installed)
once its name is typed back, before the head removes it. The habitat itself is no
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
| `up --host <name> [--user <user>] [--port <n>] [--image <tag>] [--backends own\|operator] [--name <name>] [--attach]` | Runs the habitat on another machine over SSH and keeps a pointer to it here (§ A habitat on another machine). Idempotent. A habitat already running there with no pointer here is refused unless `--attach`. `--host local` is this machine, as it is to `graft`, `place` and the host verbs: the same as `up` without `--host`, a habitat here and no SSH (`--user` and `--port`, another machine's login, are refused with it, exit 2). |
| `up [--no-body] [--image <tag>] [--profile local] [--backends own\|operator] [--name <name>]` | Births the habitat the first time (named `--name`, `local` by default; above), then brings every recorded allocation up through the `compose` provider (one with no server of its own, every part held by the habitat's servers, is reported `held by the habitat's own servers`), migrating any whose recorded schema version is behind this toolchain's, records its placement (its host, its policy, its head and face; above) and the head's URL (`head.url` and `habitat.profile`), and starts the habitat's `body` and `face` services in its compose project (`docker compose up -d`, its image `--image <tag>` or `HALE_HABITAT_IMAGE` when either names one; else, when this machine has no image of this toolchain, the release's published `ghcr.io/hale-lang/hale:<version>` pulled and used when it runs this very toolchain (`hale --version` alike); else built from the embedded `Dockerfile` and `hale.pin`), waiting on their probes: three minutes once the face has what it runs, and while the face's log says it is still building the head's programs (a first start, or one after the toolchain changed) up to twenty minutes; a face or body that does not come up is reported with its last lines of output. `--no-body` births it, brings its backing allocations up and records its placement, and starts no `body` or `face`. Idempotent, and after an `up` that failed as well (§ Long-lived, and only its acts provision). `--profile` takes `local`; any other is refused (exit 2). |
| `image build [<checkout>]` | Builds the habitat toolchain image from `<checkout>` (`.` by default) using its `Dockerfile`, tagged `hale-habitat:<pin>`, and prints the tag on stdout. |
| `brief` | Prints the habitat's brief (below): the head's `Commands::brief`, asked over the head's surface as the person this account maps to. |
| `status` | The habitat, its name, its genome, body and face (its address), its head's MCP endpoint while the face runs, the services placed on its hosts under their hosts (each with its readings endpoint), and each allocation: its organism, whether each role's service runs, the schema version memory and the nerves record against this toolchain's, how a heart's declared migration last ran, and whether its body runs (§ A sapling's body). |
| `down [--forget [--plan]]` | Takes the habitat's compose project down (`docker compose down`), stopping its `face`, its organisms' bodies and its own `body` (drained) and every service of its project and leaving only its volumes; `up` brings them back. `--forget` removes the habitat whole, planned and confirmed by its name typed back at a terminal; `--plan` prints the plan and exits (above). |
| `migrate [organism]` | Brings the organism's allocations up and migrates them: memory's schema and the nerves' stream move to this toolchain's, and the head's DSN is kept in the checkout's `.hale/dna/memory.env`. With `HALE_DNA_MEMORY_DSN_OWNER` or `HALE_DNA_NATS_URL_OWNER` set, that server is migrated instead. Idempotent; the organism defaults to the current directory. Runs the heart's declared migrations (below). |
| `sweep <root> \| --test-vaults [<root>]` | Takes down every compose project run from under `<root>`, a test's habitat, and confirms it gone by the project's label. Never the developer's habitat nor one outside `<root>`. Exit 1 names what survived. `--test-vaults` sweeps the vaults of processes that are gone. Nothing is done without docker. |
| `organism <name> dna <verb> [args…]` | `hale dna <verb>` in the grafted organism's own directory; through a pointer, on the habitat's machine over SSH (§ A habitat on another machine). |
| `graft <dir> [--attach] [--no-library] [--no-pack] [--profile local\|remote-body [--remote <url>] [--body <user@host>]] [--with <pack> [--set <param>=<value>]… [--as <who>]]` | Births an organism as the habitat's child, the one way an organism other than the habitat is born. See below. |
| `place <key>/<role> <host>` | Places an allocation on a host (`Commands::place_allocation`, above); nothing applies. |
| `apply [<host>] [--fresh]` | Applies the allocations where they are placed and prints the pulse (`Commands::apply_allocations`, above). |
| `host add <name> [--user <user>] [--port <n>]` | Adds a machine of the LAN as an `ssh` host (above): prints the host key first seen there and its fingerprint, what it offers, and the habitat's public key with the line its `~/.ssh/authorized_keys` takes. A host already added is refused (`host_exists`), and one where no SSH server answers is not added (`unreachable`). |
| `host join <name>` | Run on the machine joining, a habitat on another machine (a pointer): records this machine as a `joined` host by `name`, a name the habitat's machine reaches it at, which must resolve to this machine here (refused otherwise, and for the habitat's own machine). The habitat never logs in there: no key, no account, no probe (`host probe` is refused, `joined`); it reaches the machine on its services' ports alone, its exporters (installed here) scraped by `name`, and its GPUs' among them when `nvidia-smi -L` lists one here (the one offer a joining machine states, `nvidia-gpu`; any other it names is not taken). `--host local` on it places here; from anywhere else a placement there is refused. `host remove` drops its entry; what is placed there is taken off there, and its exporters only when run there. |
| `host probe <name>` | Probes the host again over SSH: a `pending-key` host whose key is now authorized is `ready`, with what it offers. One still waiting on its key has its setup finished the way `host add` does it (the habitat's account made, or the key appended, over the person's own login), then probed again; when that login does not reach the machine, what to do there is printed. A host that answers with another host key is refused (`host_key_changed`). |
| `organism remove <name> [--plan]` | Removes an organism the habitat grafted, planned first and confirmed by its name typed back at a terminal (above). |
| `organism upgrade <name> [--plan]` | Moves an organism grafted from a sapling to the revision the habitat pins now, keeping its record, data and allocations: the revision fetched and its digest checked, its tree moved with its graft's changes carried, its migrations run, its services re-applied, a `habitat.organism.upgraded` row recorded and its placed host commands re-run (§ Nurseries and saplings). |
| `host remove <name> [--plan]` | Removes the host, a `habitat.host` row in state `removed`, planned first: with allocations on it, the plan (each allocation and what stops or is deleted on that host) is executed only once the host's name is typed back at a terminal; `--plan` prints the plan and exits (above). `local` is not removed. |
| `graft <sapling> [--host <name>] [--own <part>[=<host>]]… [--reuse <part>]… [--plan] [--no-library] [--no-pack] [--set <param>=<value>]…` | Grafts a sapling the habitat pins, by name, its application placed on the host `--host` names (the habitat's own machine without one, as `--host local` there) and each body part filled as planned (below). |
| `nursery add <source> \| refresh \| list \| ratify <sapling>` | `add`: the Board trusts a nursery (`Commands::add_nursery`), then what it offers is read; a git nursery's source may name a revision (`<url>#<tag-or-commit>`), and adding a repository the Board trusts at another revision re-pins it: the earlier source is retired (`state: removed`, `repinned` naming the new one), so the nursery is read at the revision named last; `refresh`: every nursery read again; through a pointer, a nursery that is a directory here is staged here and shipped to the habitat's machine first, by `add` and by each `refresh` (§ Nurseries and saplings); `list`: the nurseries and saplings (`Commands::nurseries`, `::saplings`); `ratify`: the filed revision pinned (`Commands::ratify_sapling`). |
| `forge <organism> forgejo\|github\|file\|none [--repo <owner/name>] [--key-vault <slot>] [--no-head]` | Allocates the organism's forge (above): the head's `Commands::allocate_forge` while it answers, the act itself with `--no-head`. |
| `models <organism>\|--all [--plan]` | Points a grafted organism's model map (every one's and the habitat's own genome's with `--all`; the genome's alone as `models habitat`) at the habitat's models, as a graft does (`spec/dna.md`, The habitat's models): its key drawn in the provider's family and copied into its slot, the provider's reader vault restaged, the provider asked what it serves, and each category no person wrote mapped there in the name `hale habitat`; a tenant served nothing ends in the provider's `grant` as a `next` line. For an organism grafted before the provider, or a model placed since. `--plan` says what it would draw, copy and map, and does nothing. Refused when the habitat holds no models provider. Through a pointer it runs on the habitat's machine. |
| `handover [organism]` | Prints what the habitat hands the organism's heart, one `NAME=value` a line. |

The free-standing birth verbs are retired (`spec/dna.md`, Graft): each
exits 2 naming the graft that replaces it.

## The face

The habitat is served as every organism is: the standard face over its own
head (`spec/dna.md`, The habitat's face). `up` starts the `face` service
(`hale dna head <genome>`) beside `body` (`hale dna dev <genome> --no-iris`)
in the habitat's compose project, its state under `face-state` in the
habitat's directory.

**The face's assets, a dev override.** The face serves the browser assets
the toolchain embeds (`dna/face/web`, materialized beside the launcher and
rewritten at every start), read once when it starts. For working on them
against a running habitat, the launcher serves another directory instead:
`HALE_FACE_WEBROOT` when it is set, else the habitat directory's `face-web/`
(`HALE_DNA_HABITAT`, else `HOME`, which the `face` service is given) when
that directory holds an `index.html`. The directory must hold every asset
the launcher names, or the face refuses to start as it does without the
embedded ones; the launcher says `face: serving the browser assets from
<dir> (a dev override; …)` on its log when it does. So a checkout's
`dna/face/web` copied to `<habitat>/face-web/` is served after `docker
compose restart face`, and removing the directory returns the face to the
embedded assets at the next restart. Nothing else reads the directory; it
is never written by the habitat. The habitat's address, and each organism's, is a block
of 8 loopback ports recorded in `dna.head.url` (the habitat's in its
genome's, an organism's in its own, assigned at its graft); the face is the
block's base and the head the next port (`http://127.0.0.1:<head_port>`,
recorded on the `habitat.profile` row and in `<habitat>/head.url`). A block
is never assigned over a port that listens on every interface of the
machine: the senses' scrape endpoints (9464 to 9466) and the machine's node
and container exporters (`:9100`, `:9338`; a block held over one keeps the
exporter from binding, and the machine's readings down while it is held). The
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
the habitat's name, state, record revision and how many organisms it
grafted, read by the
page from the head's `Commands::index` row, then the web UIs across its
organisms (`Commands::uis`, The home screen's UIs below), through the generated ES-module
client committed beside it (`head-client.js`, from `dna/face/operator`). It
has no build step. A visitor with no session is redirected by the head's
gate before the page is served; a read refused for want of one is shown as
the crossing. The page's visual rules are CSS custom properties
(`operator.css`): near-black ground, one monospace, hairline rules, brass
only for a crossing, color only for state (up, degraded, down).

The page is one session with four screens, each a module of the same
directory, switched by the hash (`#habitat`, `#hardware`,
`#infrastructure`, `#organisms`), with no build step and no graph
renderer; its navigation also links the organization shell, the
habitat's Grafana (`/grafana/`, Readings below) and its alerts
(`#alerts`, Reflexes below). Every call is the generated client's; the page speaks no other
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
  organism, how it runs, what fills it, host, the state the provider
  observes) and the policy
  (`Commands::policy`). How an allocation runs is the row's `runs_by`:
  `compose` for the habitat's own Forgejo (its `forge` service) and
  sign-in stub (inside its `face` service), whose `provider` names the
  product (`forgejo`, `oidc-stub`) and is shown as what fills it; any
  other allocation's provider, as it is. A host picker per allocation calls
  `Commands::place_allocation`; `apply` calls `Commands::apply_allocations`,
  follows the apply through `Commands::apply_log` until it ends, and shows
  the pulse, what each provider observes of each allocation after it. A move of a memory that holds a record is refused by the row;
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

### The home screen's UIs

The first screen lists the web UIs across the habitat's organisms, not
the organisms themselves (their state and pulse are the organisms
screen's): one row each, with its label, its organism, its link
and how to sign in, from `Commands::uis` (`spec/dna.md`, The habitat's
face). A visitor signed in to the habitat is handed into a grafted
organism's face by the habitat head's `GET /organisms/<name>/face`: a
redirect to that face's address with its own launch token (read from
`<habitat>/allocations/<key>/face-state/head.token`, the head's own
directory), which opens its `/` and sets its session as `hale habitat open`
would; nobody not signed in to the habitat is handed in (401), an organism
the habitat does not hold is unknown (404), and a token the head cannot
read (a face on another machine) is a 404 naming the face's own URL. The
token is in the redirect and nowhere else. The habitat's org chart links
each child organization through it. Each organism's face is one, `<organism> face`, at the address its
graft row holds, signed in through the habitat's sign-in. Each UI the
organism's manifest declares (`[dna.ui.<name>]`: `label`, `service`,
`path`, `login`; `spec/dna.md` § Declared web UIs) is another, read from
the organism's directory as the screen asks, so an upgrade's declaration
is the one listed:

- **Its link** is the address the organism's `services` allocation
  publishes for the UI's `service`, as the organisms screen links it,
  with the UI's `path` after it. A service published on its host's
  loopback alone is shown as `loopback only on <host>`, not linked; one
  with no published address says why (`its services publish nothing (not
  applied)`, `its service <name> publishes no port`).
- **Its sign-in**, when it declares `login` (a declared secret whose
  person is `read`), is the command that reads that secret from the
  person's machine: `hale habitat organism <organism> dna secret show
  <login>`. The value is never shown.

`hale habitat graft` refuses a manifest whose UIs the toolchain refuses
(an unknown key, no `label` or `service`, a bad name, a `path` that does
not start with `/`, a `login` that names no declared secret or one whose
person is not `read`), saying why; a declaration an upgrade made refused
is listed as one row with the refusal.

## Readings: the machines' exporters and Grafana

The habitat reads the machines it runs on and charts them. Every machine
it uses runs a **Prometheus node exporter** on `:9100`, and the habitat's
senses' store scrapes each as the job `host`, its series labelled `host`
(the habitat's own machine by the name `host_label` gives it, a host by
its name). A **Grafana** behind the habitat's face charts them, its one
data source the habitat's senses' store.

**The exporter is the machine's.** One per machine, whichever habitats use
it, installed over the person's own login, where sudo can ask:

| When | Where |
|---|---|
| `hale habitat up` | this machine, the habitat's |
| `hale habitat up --host <h>` | `<h>`, over SSH from the person's machine (the run inside the habitat's image installs none) |
| `hale habitat host add <h>`, `host probe <h>` | `<h>`, over the same login its account was made over, once it is probed ready |
| `host remove <h>`, `down --forget` | taken off each, the same way; a remote forget takes it off the habitat's machine and every host it named |

On **Linux** it is the container `hale-node-exporter`
(`prom/node-exporter`, pinned), on the host's network and process
namespace, reading the root filesystem read-only, labelled
`lang.hale.node-exporter=<version>`, restarted unless stopped; an install
replaces a container of the habitat's labelled otherwise. The release is
1.11: 1.12's frequency collector also reads `cpuinfo_avg_freq`, which a
7.0 kernel answers with EAGAIN for a core that has been idle, and the
exporter's read then waits forever, so every scrape of the machine hangs. On **macOS** it is
the release's own `node_exporter` (pinned, its archive checked against the
release's SHA-256) under launchd, `lang.hale.node-exporter`: a
LaunchAgent of the person's when the login has a GUI session (no sudo),
else a LaunchDaemon run as `nobody` (sudo, at a terminal). A container
there would read the VM docker runs in, not the Mac. An exporter that
already answers on `:9100` with node readings is scraped as it is and
never touched; a removal takes off only the habitat's own (the container
by its label, the launchd job by its label). An exporter is the machine's,
one on its port, read by every habitat there: each habitat that installs
it, or finds it its own, records itself as an owner (a file named by the
habitat under `~/.local/share/hale/exporters/<node|containers|gpus>/` of
the login), and a removal takes it off only when no other habitat owns
it, else says it `stays`: forgetting one habitat never stops another's
readings. One installed before owners were recorded has none, and the
first removal takes it off. An install that cannot run
(sudo with no terminal, docker the login cannot reach, another OS) says
why and stops nothing: `up` and `host add` go on. Every download an
install makes rides out a short network blip: the macOS archive through
curl's own retries (`--retry 5 --retry-delay 2 --retry-all-errors
--connect-timeout 10`), and every image (the node exporter's, cAdvisor's,
the GPU exporter's) pulled before its container runs, unless docker has
it, in five tries two seconds apart. One that still fails stops that
exporter's install, which says why on its own line, from the tool's last
words and never as a raw line of the tool's above it, and how to retry:
`node    local: no exporter: the download from github.com failed (curl 56:
Connection died, tried 5 times before giving up); \`hale habitat up\` again
retries it` (`hale habitat host probe <h>` for a host, `host join <name>`
for a joined machine, `up --host <h>` for the habitat's machine there),
and `docker could not pull <image> (<docker's last line>)` for an image.
An install that fails otherwise says the last line of its error output.
`HALE_DNA_NODE_EXPORTER=off` turns all of it off for a run.

**Containers too.** Beside its node exporter every machine runs a
container exporter: cAdvisor (`ghcr.io/google/cadvisor`, pinned) as the
container `hale-cadvisor` wherever docker runs, a Mac's inside docker's
VM, where its containers are. It runs on the host's network on `:9338`,
privileged, reading docker's containers only (docker's socket mounted by
its own path, which a desktop VM's symlink would not resolve), keeping of
their labels only `com.docker.compose.project`,
`com.docker.compose.service`, `hale.habitat`, `hale.organism` and
`hale.role`, and collecting CPU, memory, network and OOM
events every 15 s. It is installed, found, replaced (labelled
`lang.hale.cadvisor=<image> <the labels it keeps>`, so one keeping others
is replaced; one another habitat made is shared, as its owners say) and removed where and when
the node exporter is; `HALE_DNA_NODE_EXPORTER=off` turns both off. The store
scrapes it as the job `containers` from `senses.d/containers/*.json` (every 15 s, waiting up to 14 s for cAdvisor's answer, which renders every container's series and outlasts the store's default 5 s on a busy machine; `up` gives a job an earlier toolchain wrote the same timeout),
which `up` adds to the habitat's store configuration when it has none.

**GPUs too.** A Linux machine with an NVIDIA GPU (`nvidia-smi -L` lists
one) also runs a GPU exporter: `nvidia_gpu_exporter` 1.17.0
(`utkuozdemir/nvidia_gpu_exporter`, the plain image, which reads
`nvidia-smi`, pinned by its tag and the digest of the release's image
index, so docker refuses any other content) as the container
`hale-gpu-exporter`, on the host's network on `:9835`, given the GPU by
docker's NVIDIA runtime (`--gpus all`, the `utility` capability, which
puts the host's `nvidia-smi` into it). It is installed, found (an
exporter already answering on `:9835` with `nvidia_smi_` readings is
scraped as it is), replaced (labelled `lang.hale.gpu-exporter=<image>`)
and removed where and when the node exporter is, owners and all;
`HALE_DNA_NODE_EXPORTER=off` turns it off with the others. A machine
without such a GPU, and every Mac, runs none, and its install says
nothing; one whose docker cannot give a container the GPU (no NVIDIA
Container Toolkit) says so and runs none. The store scrapes it as the job
`gpus` (every 15 s) from `senses.d/gpus/*.json`, which `up` adds to its
configuration when it has none: `gpus/hosts.json`, which the head writes
from the `habitat.host` rows that offer `nvidia-gpu` (below), and
`gpus/local.json`, the habitat's own machine, which `up` writes while a
GPU exporter answers there (as its install found it; the run inside the
habitat's image on a machine reached with `--host`, which installs
nothing, while docker there runs the habitat's GPU exporter or has the
NVIDIA runtime) and removes otherwise. Its series are the exporter's
`nvidia_smi_*`, each labelled with the GPU's `uuid`; a field
`nvidia-smi` reports as `[N/A]` is not exported (a GB10, whose memory is
the machine's, reports no memory of its own).

**Placed services too.** A service an organism's host command left on a
host (§ Host commands) whose declaration names `readings` is scraped
where it serves them, never polled: when the head records the placement
(`Commands::record_placed`) it writes the file_sd file
`senses.d/services/<organism>-<command>-<id>.json` in the habitat's senses
allocation, written whole and renamed into place, its targets
`["<host address>:<readings>"]` (a host's address as its row names it;
the habitat's own machine `host.docker.internal`), labelled `host` (the
host as the habitat names it), `organism`, `service` and `kind`
(`systemd` on Linux, `launchd` on macOS); one target group per host when
the same organism, command and id are placed on several. `unplace`
(`Commands::record_unplaced`) writes it again without that host and
removes it with the last, and a host's removal rewrites them all. The
store scrapes the directory as the job `services` (every 15 s), which
`up` adds to its configuration when it has none, as it adds
`containers`.

**Whose a container is.** Every container the habitat runs carries three
labels: `hale.habitat` (the habitat's name), `hale.organism` (the organism
it is a part of, by its directory's name; the habitat's own genome is
`habitat`) and `hale.role` (the part: `memory`, `nerves`, `senses`,
`body`, `face`, `grafana`, `forge`, `exporter`, and `services` for an
organism's own compose services). The habitat writes them into the
compose files it owns (an allocation's, the forge's, the body and face's,
Grafana's) and onto the exporters it runs. An organism's own compose
files are never touched: when its services are applied, the habitat asks
compose how it reads those files (`docker compose config --format json`,
with the environment, else `--no-interpolate`) and adds an override of its
own labelling each service and giving each long-running one its restart
policy (**Kept running**, above), kept under
`<habitat>/labels/<project>.yaml` and, on an SSH host, beside the
habitat's copy of the render (`.hale-labels.yaml`); the images the files
name, `${HALE_DNA_REVISION}` among them, are left as they are. A habitat's compose files are rewritten whenever
their text changes, so compose recreates a service whose labels moved and
keeps its volume.

**What is scraped follows the record.** The store reads
`senses.d/*.json` beside its allocation (mounted as a directory, so a file
renamed into it is seen): `up` writes `hosts.json` (and `containers/targets.json`, at `:9338`) with this machine
(`host.docker.internal:9100`, the store's container reaching it over the
host gateway) when there is none, and the head rewrites it whole from the
`habitat.host` rows whenever one is written (`add_host`, `probe_host`) and
after a host's removal, each host at `<address>:9100`, and
`gpus/hosts.json` beside it, each host that offers `nvidia-gpu` at
`<address>:9835` (a host probed before its GPU was looked for is listed
once `host probe` finds it). A grafted
organism's store has the same job and an empty directory.

**Grafana is behind the face.** `up` makes the compose service `grafana`
(`grafana/grafana`, pinned) in the face's network namespace, on the
loopback at the head block's sixth port (`GRAFANA_OFFSET` 5; the block is
published on macOS but for that port), served under `/grafana/` of the
face. Its provisioning (`<habitat>/grafana/provisioning`, written at
every `up`, a file only when its content differs) is the habitat's
senses' store as its one data source (uid `habitat-senses`) and the
habitat's dashboards. The dashboards' provider reads its files again every
10 seconds (`updateIntervalSeconds`), and the home dashboard
(`GF_DASHBOARDS_DEFAULT_HOME_DASHBOARD_PATH`, `hosts.json`) is read at each
request; a Grafana that `up` leaves running (neither made again nor
stopped) is restarted when a provisioning file changed, and one `up` makes
reads them at its start, so a toolchain's changed dashboard is what Grafana
serves after `up`. An `up` that changed none of them restarts nothing. `Services` (uid
`hale-containers`, the Containers dashboard it replaced) shows what runs
where, whatever a host's system: one table and one set of graphs for the
containers cAdvisor reads (`kind` `container`, with the organism and part
their `hale.*` labels name, and their image) and the services placed on
the hosts (`kind` `systemd` or `launchd`, read from their own readings:
`up`, `hale_process_resident_memory_bytes`,
`rate(hale_process_cpu_seconds_total)` and `time() -
hale_process_start_time_seconds`). Each query joins the two with PromQL
`or`, `label_replace` giving both the same labels (`kind`, `host`,
`organism`, `part`, a `name` that is the container's name or the service);
the table's rows are each one's state (a placed service whose readings do
not answer is `up` 0, and shows as down rather than vanishing), uptime,
memory and CPU; its graphs CPU and memory of both, the containers'
network, and the restarts (both) and OOM kills (containers) in the
window, with each machine's running count. It shows the habitat's
containers by default (`show`: the habitat's, those carrying
`hale.habitat`; or everything on the host, which a host's other workloads
compete with; a placed service is always the habitat's) and filters by
organism and host, both kinds alike. `Hosts` is its home: at its top a
table, one row a machine, of its memory and main disk (`/`, a Mac's
`/System/Volumes/Data`), each total, in use and free and the share used,
and its CPU busy over 5
minutes, memory in use read as `memory-pressure` reads it, and its
NVIDIA GPUs: busy (their average utilization), memory and memory in use
(their framebuffers summed, shown only when every GPU of the machine
reports its own: a GB10's is blank, its memory the machine's), the
hottest's temperature and their power draw summed, blank on a machine
without one. Each share (memory and disk used, CPU and GPU busy) is a
percent with a gauge of it in the column after (amber from 75%, red from
90%); the gauge shows no value of its own, so its bar is the whole cell
and every row's starts and ends alike. Then whether
each machine answers, its uptime, CPUs and memory, and over time CPU busy,
load per core, memory used, disk used, network and disk throughput, for
Linux and macOS alike, and a Linux machine's CPU frequency. Its data is `<habitat>/grafana/data`. The operator
face's navigation links it as `dashboards`.

Grafana signs in no one itself: its login form, basic authentication and
anonymous access are off, and its admin password is drawn at each `up`.
It trusts the person's name on one header (Grafana's proxy
authentication), whose name is drawn once per habitat (`X-Hale-Grafana-`
and 32 hex digits, in `<habitat>/grafana.secrets`, 0600) and known only to
the face and Grafana. For every request under `/grafana/` but Grafana's
public build (`/grafana/public/`, the same for everyone), the face asks
the head's gate at `/` as the browser's session, exactly as for the
shell: a refusal is passed on as it came, and a 204 names the person
(`X-Hale-Person`), whom the face relays to Grafana under the header. The
browser's own `X-Hale-*` and `Authorization` lines never reach Grafana. A
person is signed up in Grafana at their first visit, as an Editor.
`HALE_DNA_GRAFANA=off` runs none, and a habitat with no senses' store of
its own (its allocation publishes none) runs none and says so.

**Grafana runs while it is used.** By default `up` creates the service
(its image pulled, joined to the face's network; one an earlier `up` made
is removed first) and does not start it: `up` names the directory of the
habitat's compose files to the face (`HALE_DNA_GRAFANA_ON_DEMAND`). Every
signed-in request under `/grafana/` stamps its time
(`<habitat>/grafana/last-use`). While Grafana does not answer on its port,
the face answers such a request with a page that reloads itself every 3 s
(503, `Refresh` and `Retry-After`; a request that is not a page, under
`/grafana/api/` or `/grafana/public/` or not a `GET`, gets the 503 refusal
`grafana_starting`), and the face's keeper, once a second, starts it
(`docker compose start grafana`, which makes nothing) for a use in the
last 30 s. A start compose refuses, or one Grafana does not answer within
60 s, is written to `<habitat>/grafana/start.err` and shown on that page,
which then stops reloading; reloading it tries again, and the file goes
once Grafana answers. The keeper stops Grafana (`docker compose stop
grafana`) once the later of its last use and the face's own start is 15
minutes past, once; an open dashboard, which refreshes itself every 30 s,
is use. `HALE_DNA_GRAFANA=on` runs it from `up` to `down` instead, and
the face neither starts nor stops it.

**Grafana's footprint is bounded.** Its container is capped
(`mem_limit: 384m`), its Go runtime collects under `GOMEMLIMIT=200MiB`,
it fetches no plugin at its start (`GF_PLUGINS_PREINSTALL_DISABLED`), the
data sources and apps it ships that the habitat never uses are disabled
(`GF_PLUGINS_DISABLE_PLUGINS`: each data source with a backend is a
process of its own), and its own alerting is off (the habitat's reflexes
are its senses' rules). Running, it holds about 170 MiB with both
dashboards' queries answered; with Grafana's defaults it held 640 MiB.
The face and body are not capped: a face whose image does not already
hold the head's programs builds them, in parallel, and that takes
several GB. A habitat's whole budget, and what a small machine gives it,
is the guide's (`docs/src/habitat.md`, Running on a small machine).

### Reflexes: what an organism watches for

A reflex is a condition over an organism's readings that someone should
know about. It is one model whatever declares it and wherever it runs:
a name, a condition (`when`, PromQL over the organism's senses), how
long it must hold (`for`), how long it keeps firing once it stops
holding (`keep`), a severity (`warning`, `critical`), the action it
names (`concern`, `restart`, `notify`) and a summary whose `{label}` and
`{value}` are filled from the alert. The manifest's `[dna.reflex.<name>]`
declares one today:

```toml
[dna.reflex.disk-filling]
when     = '100 * (1 - node_filesystem_avail_bytes / node_filesystem_size_bytes) > 90'
for      = "10m"
keep     = "10m"
severity = "warning"
action   = "concern"
summary  = "{host}:{mountpoint} is {value}% full"
```

Its evaluator is a Prometheus store: the model renders to a standard
alerting rule group (each rule named for the reflex, labelled with
`reflex`, `organism`, `severity` and `action`), so a heart run by hand
hands the same rules to its own Prometheus as a habitat does to its
senses' store. `hale habitat up` renders the habitat's own reflexes (its
defaults, then its genome's declarations, which replace a default of the
same name) into its store's `senses.d/reflexes.rules.yml`, makes the
store's configuration read `senses.d/*.rules.yml`, and restarts the store
when either moved. Its defaults watch every machine it uses (a genome's `[dna.reflex.<name>]` of the same name replaces one, a threshold or a duration of its own):

| Reflex | Fires when |
|---|---|
| `disk-filling` | a real filesystem is over 90% full for 10 minutes (keeps firing 10 minutes after it drops below) |
| `disk-full-within-a-day` | at its last six hours' rate a real filesystem fills within a day, for an hour |
| `memory-pressure` | over 90% of a machine's memory is in use for 15 minutes: all but what Linux says is available, or a Mac's active, wired and compressed pages, as the Hosts dashboard reads it (keeps firing 10 minutes after it drops below) |
| `cpu-saturated` | a machine's CPUs are over 90% busy, averaged over 5 minutes, for 15 minutes (keeps firing 10 minutes after it drops below) |

`Commands::alerts`, a read, answers the store's alerts as it holds them:
each with its labels, summary, state (`pending` while its condition holds
but not yet for its time, `firing` after) and since when. The operator
face's **alerts** screen lists them, firing first. Nothing acts on a
reflex yet: its `action` is carried on the alert for the receiver that
will act on it, and an alert is never recorded.

## Host commands: an organism's own, on a host

An organism may declare commands of its own that the habitat runs on a
host it manages, in its manifest under `[dna.host.<name>]`. The habitat
knows nothing of what a command does: voice's `node` installs whatever a
machine needs to serve models, directly on the host, outside compose.

```toml
[dna.host.node]
platforms = ["linux", "macos"]   # where it runs; empty: anywhere
root = true                      # run under the person's sudo
install   = ["hale", "run", "node", "install", "--id", "{id}", "--api", "{api_url}"]
uninstall = ["hale", "run", "node", "uninstall", "--id", "{id}"]
inputs = ["id", "api_url"]
api_service = "api"                           # whose published address is {api_url} (default "api")
service = "voice-node-{id}"                   # what the install leaves running
readings = 9464                               # where that service serves its readings
requires = { linux = ["docker", "gpu"], macos = ["gpu"] }   # what the host must have

[dna.secrets.operator-token]                  # a secret the command reads (spec/dna.md, Declared secrets)
kind = "drawn"
readers = ["api", "host.node"]
```

`hale habitat organism place <organism> <command> --host <h> [--id <id>]
[--plan]` runs `install`, and `unplace` runs `uninstall`. Both need a
grafted organism that came from a sapling the habitat pins, and a
`ready` host or `local`:

- **The habitat's own machine, and this one.** `local` is the machine
  the person types on. On a habitat on this machine it is the habitat's
  own machine, the host its rows already call `local` (no `host add`
  makes it, and no SSH to oneself is needed): every script below runs
  here, with `sh -c`, as the person (sudo asks at this terminal), without
  this process's `HALE_*` and `LOTUS_*` variables, as a login there starts
  without them. Through a pointer (a habitat on another machine, `up
  --host`) `local` is still this machine: the host the habitat added that
  is it (one whose address resolves to an address of this machine's), its
  scripts run here, the placement recorded under that host's name; when
  the habitat added none, it is refused, naming the one `hale habitat host
  add <name>` that makes it one. The habitat's own machine is then said by
  its own name, whole or by its first label (`--host rack-a.lan` or
  `--host rack-a` for a habitat `up --host rack-a.lan`), reached over the
  person's login the pointer names (its host, user and SSH port), recorded
  as `local` and said by that name; its default `{id}` is that name's first
  label. A host the habitat added keeps its own name first. The scripts,
  the plan, the inputs and the record are the same as for any host; the
  plan says which way it is reached. Any other name is a host the habitat
  added: one it does not hold is refused, naming `hale habitat host add
  <h>`. A graft's `--host` and `--own <part>=<h>` resolve their names the
  same way, on the machine the person types on.

- **Its source.** The organism's sapling repository at its pinned
  revision, cloned (then fetched) on the host under the login's
  `~/.local/share/hale/hosted/<organism>`, where the command runs. Its
  declaration is read from that checkout, so the command is the one the
  pinned revision declares.
- **What the host is.** Before anything is touched, the verb reads the host
  over the login: its OS and architecture, its package manager (apt-get,
  dnf, Homebrew), what of the toolchain's prerequisites it has, the
  toolchains the habitat installed there, and the facts `requires` is
  checked against.
- **What any toolchain builds with**, the same for every organism: git,
  curl, tar, clang, and OpenSSL's and zlib's headers (Homebrew's
  `openssl@3` on macOS, where clang and zlib are the Command Line Tools').
  What the host lacks is installed first, with its package manager: under
  the person's sudo for apt-get and dnf, so only at a terminal, and as the
  login for Homebrew. A host with no package manager the habitat knows, or
  a Mac without the Command Line Tools, is refused, naming what it lacks.
- **Its toolchain.** The hale release the organism declares (`[dna]
  toolchain = "<version>"` in its manifest, as a codebase's `toolchain` is),
  else the habitat's own, installed from that release under the login's
  `~/.local/share/hale/toolchains/<tag>/`, checked against the release's
  sha256 and moved into place whole, one directory per version, never the
  person's own `hale`. A command whose first word is `hale` runs with it by
  its absolute path.
- **What it requires**, by platform (`requires = { linux = [...], macos =
  [...] }`; a list is the same on every platform), from a vocabulary the
  habitat checks from the host's facts: `docker` (docker answers the login), `systemd` (a Linux
  host whose system services run under systemd), `nvidia-gpu`
  (`nvidia-smi` lists a GPU), `apple-silicon` (an arm64 Mac), `gpu` (either
  of the two, or on Linux an AMD GPU, by ROCm's `/dev/kfd` or a render node
  whose vendor is AMD's, or an Intel render node, as a ROCm or Vulkan
  runtime uses them). A host that lacks one is refused before anything is
  installed, naming what it lacks; a requirement outside the vocabulary
  refuses the declaration.
- **Its inputs**, each named in `inputs` and filled where its argv says
  `{input}` (an input it does not declare, or one with no value, is
  refused): `id` (`--id`, by default the host's first label; `local` on
  the habitat's own machine) and `api_url` (the published address of
  `api_service` in the organism's `services` allocation, as the head
  links it, which on the habitat's own machine may be the loopback: the
  node runs on the same machine). The `operator_token` key and the
  `operator_token_file` input are refused: they are superseded by
  `[dna.secrets]`, where the seed declares the secret and names
  `host.<command>` among its readers.
- **Its secrets.** Each secret the seed declares with `host.<command>`
  among its `readers` (spec/dna.md § Declared secrets) is read from the
  habitat's vault — this machine's, or, through a pointer, the vault on
  the habitat's machine, over the person's login after the pointer's
  record is checked there — into this process's memory alone, and written
  over the person's login to the target, on the login's stdin, into the
  organism's vault directory there, `~/.local/share/hale/hosted/<organism>.vault`
  (mode 700, each entry 600, named as in the habitat's vault). The command
  runs with `HALE_VAULT_DIR` naming that directory and, per secret,
  `HALE_SECRET_<NAME>` naming its entry (a family's: each member's entry,
  one a line, in their labels' order), passed through `env` on its command
  line so that sudo, which resets the environment, keeps them. Every run,
  whatever it reads, is handed the habitat's name the same way,
  `HALE_DNA_HABITAT_NAME` (the habitat the pointer names, when the
  command is run through one). The entries
  are for the run alone: removed however it ends, and the directory once
  empty, so a command that needs one later keeps its own copy. A value is
  never on a command line, on this machine's disk, or in the head. A
  required secret the vault does not hold — a slot no person filled, a
  family with no member, a drawn secret never drawn — refuses `place`,
  `--plan` included, naming how it is filled. An optional one (`optional =
  true`) never refuses: an unset optional slot is not handed (no
  `HALE_SECRET_<NAME>` at all), an optional family with no member is
  handed empty (its variable set, to nothing). `unplace` is never refused
  for a missing secret: it hands what the vault holds and goes on without
  the rest, a required family with none handed empty. `--plan` lists the
  secrets it hands over by name (a family with its members' labels) and
  each it skips, and why, never a value.
- **The habitat's share of the host.** Every run is also handed
  `HALE_DNA_HOST_RESERVED_BYTES`, the same way: the bytes of the host's
  memory the habitat's own containers there may hold, the sum of their
  compose `mem_limit`s as the habitat wrote them (`512m`, `1g`, `128M` or
  a bare number of bytes, as compose reads them; a service that declares
  none counts 0), `0` when the habitat runs nothing there that declares
  one. On the habitat's own machine (`local`, or the host the pointer
  names) that is every service of the habitat's own allocation (its
  memory, its nerves, its senses' store, `body`, `face`, and Grafana,
  counted whether or not it runs, since it starts on demand) and its
  forge; `body` and `face` declare no limit (§ Readings), so they add
  nothing. On any host it adds each organism's memory, nerves and senses'
  store applied there (`allocations/<key>/placed.<role>`, else the
  habitat's machine), unless the habitat's own server holds that part
  for it, and an organism's own services applied there, from the compose
  file at its root. The files are read where they are, under the
  habitat's directory on its machine: here, or over the person's login
  through a pointer, after the pointer's record is checked there, as the
  vault is. On a Mac docker runs in a VM the Mac gives all of its memory
  to, whatever the containers use: there the figure is the VM's memory
  (`docker info`'s `MemTotal`, read on the host with its facts) when that
  is more than the limits. The plan names the figure (`reserve`), and
  which it is. A command that sizes
  itself by the host's memory subtracts it: a provider's node offers the
  models what is left, so a node placed on a host the habitat also runs
  on does not promise the models memory the habitat's containers may
  take.
- **Who runs it.** Everything goes over the person's own SSH logins, as
  `host add` makes the habitat's account (on the habitat's own machine,
  here or over the pointer's login, as above); `root = true` runs it under the
  person's sudo, so only at a terminal. `--plan` prints what would run,
  where, and with what (the packages it would install, the toolchain and
  whether it is there), and touches nothing on the host but the checkout;
  a host without git stops the plan at installing it, since the
  declaration is read from the checkout.
- **What it leaves running.** `service` names the system service the
  install leaves on the host, outside compose (a systemd unit on Linux, a
  launchd service in the system domain on macOS), its `{id}` filled as
  the command's is: letters, digits, `-`, `_`, `.`, `@`. The plan names
  it, and once the command exits 0 the host's own service manager is
  asked for it: `running` after `place`, `absent` after `unplace`, and
  anything else fails the verb, naming what it found.
- **Its readings.** `readings = <port>` (optional; an integer 1 to
  65535, anything else refuses the declaration) says the service serves
  its Prometheus readings at `http://<host>:<port>/metrics`: a Hale
  program's `std::observe::Scrape`, whose runtime series (memory, CPU
  time, start time) every such program serves. `--plan` and `place` name
  the endpoint they register with the habitat's senses, `unplace` the one
  it unregisters (below, § Readings).
- **Its record.** Once `place` ran and its service runs, the CLI records
  the placement through the habitat's head, as every habitat act is a row:
  `Commands::record_placed` (`{organism, command, host, id, service,
  platform, readings, toolchain}`, the Board's) appends a `habitat.placed`
  row keyed `<organism>/<command>/<host>/<id>`, with the scrape target its
  readings are registered at; after `unplace`, `Commands::record_unplaced`
  (`{organism, command, host, id}`) appends `habitat.unplaced`, keeping
  what the placement said. `Commands::placements`, a read, answers the
  placements that hold (each key's latest row; an unplaced one dropped)
  as `placements`, and every placement the record names with its `state`
  (`placed` or `unplaced`) as `history`: what the habitat ever installed
  where. `local` is recorded as the host of a placement on the habitat's
  own machine, and the head holds it without a `habitat.host` row, its
  readings scraped at `host.docker.internal`. A refused record (a host the
  habitat does not hold, a name that
  is not a word of letters, digits, `.`, `_` and `-`) fails the verb after
  the command ran, saying so. `hale habitat status` lists the placements
  under their hosts, one line each: the organism's command and id, its
  service and kind, the host, and its readings endpoint or `no readings`.
- **Taken off with the host or the habitat.** `host remove`, `organism
  remove` and `down --forget` run each placement they remove's own
  `uninstall` first, as `unplace` does (over the person's login, or on the
  habitat's own machine as `place` reaches it, under their sudo at a
  terminal), then remove what placing installed on that
  host: the checkout `~/.local/share/hale/hosted/<organism>` and the
  toolchain directory, each only when no placement left there uses it
  (from every placement the record names there, unplaced ones included),
  and the habitat's directories there left empty (§ Destroying). A
  placement that cannot be taken off (its host does not answer, sudo has
  no terminal, its uninstall fails) is said, with the exact `unplace`
  that takes it off, and the rest goes on: it fails nothing else.

Not yet: the infrastructure screen does not show the placements, and a
vault served over HTTP (`HALE_VAULT_ADDR`) hands a host command no secret.

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
calls `Commands::place_allocation`; `apply` calls `Commands::apply_allocations`,
follows the apply through `Commands::apply_log` until it ends, and shows its
pulse, what each provider observes of each allocation after it.
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
  `Commands::index`, `::uis`, `::hosts`, `::allocations`, `::policy`,
  `::nurseries` and `::saplings`, none of which asks a model.
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

The `[dna]` table is read forward-compatibly: a key this toolchain does
not know (one a newer toolchain reads, or a misspelling) is ignored with
a warning naming it (`this toolchain does not know [dna] <key> and
ignores it`), never refused, so an organism may declare what a newer
habitat reads before the toolchain beside it knows it. The keys it knows
are read and checked as each section says (`needs`, `brings`,
`migrations`, `codebase`, `fleet`, `record`, `reflex`, `secrets`, `ui`
(the web UIs the home screen lists, The face), and `host`, which this
toolchain carries for a habitat that runs host commands).

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
habitat removes them with its memory. Such an organism's memory, and a
nerves held as an account on the habitat's NATS, is never brought up as a
server of its own: `apply`, and a host's re-apply of its allocations, pass
over it (`placed_apply`), since the habitat's servers are its and are up
with the habitat. A reused `nerves` is an account of
the organism's own on the habitat's NATS (`ORG_<KEY>`, its users keyed by
its record, JetStream its own; spec/dna.md, The nerves), on no server of
its own: its account and secrets are files of the habitat server's
`nerves.d/`, which the server's configuration includes, and the server
restarts to read them. Removing the organism drops its account there, its
users with it, and its stream's data off the server's volume
(`nerves-drop`), and keeps the server; forgetting the habitat removes
them with its nerves. Neither a reused memory nor a reused nerves is ever
brought up as a server of the organism's own (`placed_apply`). A reused
`senses` is a server of its own on the habitat's machine. The application plane is the organism's own
software: its compose services, placed on the `--host` machine (the
habitat's own without one), and the
heart roles it needs (`database`, `broker`), which are the application's
own when its manifest says it brings them (`[dna] brings = ["database",
"broker"]`, a subset of its needs: its compose provides them) and are
otherwise provisioned by the habitat. `--reuse <part>` has the habitat
provide a heart role an application brings; `--own <part>` leaves it to
the application.

**An application's compose is told which heart roles it provides
itself.** Wherever the habitat brings up an organism's own compose project
(its services allocation, applied at its graft, at `hale habitat apply`,
at a host's re-apply and at an upgrade, through the `compose` provider here
and the `ssh` provider on a host), compose runs with `COMPOSE_PROFILES` set
to the application-plane roles the application provides itself under the
choices its graft kept (`parts.json`, `own`): `database`, `broker`,
comma-joined in that order, or `none` when it provides neither (an application
that brings nothing, or has no `brings`, among them). So a service under
the profile `database` or `broker` runs only when the application provides
that role itself; a service under no profile always runs; and `none` is a
sentinel, a profile no service may use (an empty `COMPOSE_PROFILES` may be
read as unset). The variable is in the environment compose runs under here
and on its command line on an `ssh` host, where it wins over a `.env` at the
application's root that sets `COMPOSE_PROFILES` (an application may ship
one naming both roles, its default for a toolchain that hands none). An
organism grafted before its choices were kept has no `parts.json` and is
handed none. Narrowing the profiles does not stop what an earlier apply
started under a wider set; a `down` takes the whole project down. `graft
--plan` names the value on its `services` line.

**`--reuse <role>` is refused when the application's compose would never
read the role, or could not reach it.** For a role the application brings
(its compose runs its own), `--reuse` has the habitat provision one and
hand it to its services (The handover: `HALE_DNA_DATABASE_DSN`,
`HALE_DNA_BROKER_URL`); a compose file that never reads it would leave that
server unused. So a graft whose compose file
(`compose.yaml`, else `compose.yml`, `docker-compose.yaml`,
`docker-compose.yml`) never names `HALE_DNA_DATABASE_DSN` (for `--reuse
database`) or `HALE_DNA_BROKER_URL` (for `--reuse broker`) is refused before
anything is made, naming the variable and `--own <role>`; `--plan` prints
the plan and the refusal and exits 1. The check is textual: the variable's
name as a whole word on a line of the file that is not a comment
(`${VAR}`, `${VAR:-…}`, `$VAR`, or `VAR` passed through in an
`environment:` entry). A graft whose services go on another machine
(`--host` an `ssh` host) is refused the same way for a role it brings:
the habitat's servers are reached from the habitat's own machine alone
(not built yet), so `--own <role>` or a graft onto the habitat's machine.
A role the application does not bring, an application with no compose
file, and a graft that goes on where it stopped (checked when it began) are
not checked.

`graft --plan` prints the plan, part by part (each
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
standard`). Last, once a sapling's own services are placed and applied, it
points the organism's model map at the habitat's models when the habitat
holds a models provider whose services are applied: a key drawn for it in
the provider's family and copied into its slot `model-HABITAT_<KEY>`, the
provider asked what it serves, and each category no person wrote mapped
there in the name `hale habitat`; grafting the provider itself does this
for every organism the habitat holds (`spec/dna.md`, The habitat's
models). Nothing of it fails the graft: what stopped it is said. A graft
of an organism that provides models while the habitat holds another
provider is refused, naming it. A graft of an organism called `habitat`
(its directory's name, or a sapling's) is refused before anything is
made: the habitat's own genome is a tenant of its models under that
label, and a provider names what it keeps for each by its label. `<dir>` is a new application's directory or, with `--attach`,
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
| `habitat.allocated <id>/<role>` | the first time a role is provisioned for an organism; for the habitat's own `head` and `face`, at `up` whenever their placement changes; an organism's `body` at its graft, or at the first `up` or apply that keeps it (§ A sapling's body) | `organism`, `role`, `service`, `project`, `provider`, `host`, `vault` (slots held by name, never a value), and for the head and the face their `address` (`""` while none is assigned) |
| `habitat.host <name>` | at `up` for `local`, at `host add`, `host probe` and `host remove` for an `ssh` host, and at `host join` and `host remove` for a `joined` one, whenever what it says changes | `name`, `provider`, `offers` (space-separated), `state` (`ready`, `pending-key` or `removed`), `address`, `port`, `user`, `host_key` (the public host key first seen, `<type> <base64>`), `fingerprint`, `os`, `arch` |
| `habitat.policy <role>` | at `up`, whenever the role's provider changes | `role`, `provider`, `profile`, `backends` |
| `habitat.migrated <id>/<role>` | each time an organism role's schema version moves | `organism`, `role`, `from`, `schema_version` (memory's `MEMORY_SCHEMA_VERSION`, the nerves' `NERVES_STREAM_VERSION`) |
| `habitat.migrated <id>/<role>` | each run of a heart's declared migration | `organism`, `role`, `command`, `status`, `output` (a heart role carries no version) |
| `habitat.profile local` | at birth | `profile`, `backends`, `project`, `name`, `face`, `head` |
| `habitat.grafted <child>` | at a graft | `child`, `parent`, `needs`, `backends`, `name` (the directory), `face` and `head` (the organism's addresses, recorded under the habitat's declared address, `HALE_DNA_HABITAT_ADDRESS`: `127.0.0.1` locally, the host's name on another machine, so the operator face's links reach them from the admin machine; `hale dna head` under such a habitat, as `hale habitat organism <name> dna head .` runs it there, binds the habitat's listen address and answers under its declared address, as the habitat's own face does, and keeps its state in a directory of the organism's own, `organisms/<record>` under the state directory every run there shares), `purpose` (its declared purpose), and for a sapling's graft `sapling`, `revision`, `digest` and `nursery` |
| `habitat.applied <key>/<role>` | at each apply of an allocation | `host`, `provider`, `ok`, `why`, `state` (the pulse after), `fresh` |
| `habitat.nursery <source>` | when the Board trusts a nursery | `source`, `kind` (`dir` or `git`), `state` (`trusted`) |
| `habitat.sapling <name>` | when a nursery's sapling is first read, and at its ratification | `name`, `purpose`, `repo`, `revision`, `digest`, `packs`, `set`, `nursery` |
| `habitat.sapling.proposed <name>` | when a nursery offers another revision of a pinned sapling, once | the sapling's fields at the new revision, and `from` (the pinned revision) |
| `habitat.organism.upgraded <key>` | when `organism upgrade` has moved a grafted organism to its sapling's pinned revision | `organism`, `child`, `sapling`, `from`, `from_digest`, `to`, `digest` (the tree's), `commit` |

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
its environment, at every graft, `hale habitat migrate`, `hale dna
upgrade` and `hale habitat organism upgrade` (at the revision it moved to). The tool is idempotent and keeps its own ledger; the habitat
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

**An organism's own services are handed the heart roles the habitat
provides them.** For each role its graft had the habitat provide (kept
`reuse` in `parts.json`, and allocated), the habitat's apply of its
services on the habitat's own machine (at the graft, `apply`, a re-apply
and an upgrade) hands its compose project that role's lines of the table
above, in the environment compose runs under, as it hands
`COMPOSE_PROFILES`. The servers are the habitat's containers, published on
its machine's loopback alone, which another compose project's containers
do not reach; so the habitat makes a network of the organism's own,
`<project>-hale-roles` (labelled as its services' compose project), joins
the server holding each role to it before the services come up, under the
alias `hale-database` (its memory server) or `hale-broker` (its nerves
server), and its override joins each of the services to it beside the
networks they are on (one with a `network_mode` is left alone). So
`HALE_DNA_DATABASE_DSN` names `hale-database:5432` and
`HALE_DNA_BROKER_URL` is `nats://hale-broker:4222`. A node of the
application is a process of a machine, on no such network, so the broker's
services are also handed `HALE_DNA_BROKER_NODE_URL`, the server as it is
published on the habitat's machine (`nats://127.0.0.1:<port>`), for a node
there to dial. Each role's vault
entries (`postgres-heart_<id>`, and the broker's two) are copied into the
organism's reader vault beside its declared secrets, and `HALE_VAULT_DIR`
names it, so a service that mounts it reads the passwords the DSN and the
`_VAULT` lines name. A withdrawal takes the network down with the services,
the servers taken off it first, and so do `organism remove`, `down
--forget` and `hale habitat sweep`. A server recreated since (a toolchain
that changed its compose text) is off the network until the next apply of
the services (`hale habitat apply`) joins it again. Services placed on an
`ssh` host are handed none of this: the habitat's servers are not reached
from another machine (a node of the application on another machine
neither; not built yet).

Every compose project the habitat applies on an `ssh` host (an organism's
own services placed there, an allocation placed there) is also handed two
facts of the host in its environment, whatever it runs; the habitat knows
none of the project's own variables, and a project maps these in its
compose file. On this machine neither is set.

| Variable | Value |
|---|---|
| `HALE_DNA_HOST_NAME` | the host's name, as it was added (`hale habitat host add <name>`) and as the LAN's DNS resolves it |
| `HALE_DNA_HOST_LISTEN` | the host's listen address: the first IPv4 address its name resolves to on the habitat's machine, through the toolchain's own resolver (`hale habitat resolve <name>`), which the LAN reaches it at; an apply on a host whose name resolves to none is refused, saying so |

An organism's own services, applied here or on an `ssh` host, are also
handed the secrets the seed declares for its programs (spec/dna.md §
Declared secrets), when its programs read any: `HALE_VAULT_DIR`, the
organism's reader vault where the services run
(`<habitat>/allocations/<id>/vault` here, `~/.hale/habitat/<project>/.vault`
on a host, written before compose runs and removed from a host with the
services), and `HALE_SECRET_<NAME>` per secret, naming its entry there (a
family's: each member's entry, one a line). The compose file passes them
to its services and mounts the directory; the habitat never edits it.

An organism's own services are also handed the habitat's name,
`HALE_DNA_HABITAT_NAME` (its `name` file, as `hale habitat up --name`
set it; `local` when it was born with none), wherever they are applied
and whatever their programs read: in the environment compose runs under
here, and on the command line before compose on an `ssh` host,
shell-quoted like the secrets. A repository body (`hale dna dev`, `run`)
hands its compose services the same when the habitat grafted the
organism (its parts' choices are kept beside its allocations,
`parts.json`), and nothing of a habitat otherwise. Like the secrets, it
reaches a service only as the compose file passes it
(`environment: { HALE_DNA_HABITAT_NAME: "${HALE_DNA_HABITAT_NAME}" }`); a
provider names what it keeps for the habitat it serves by it, as voice
names a tenant's project `<habitat>-<label>` (`spec/dna.md`, The
habitat's models).

They are also handed `COMPOSE_PROFILES`, the heart roles the application
provides itself (Graft and `[dna] needs`), the same way.

They are also handed `HALE_DNA_REVISION`, the full commit the
organism's checkout is at (its sapling's pinned revision), the same way.
A compose file that names its images by it (`image:
ghcr.io/<org>/<app>:${HALE_DNA_REVISION:-dev}` beside its `build:`) has
them pulled, not built: the habitat applies such a render without
`--build`, and compose pulls the revision's published image, building
from the context only when none is published (the compose
specification's pull-then-build for a service with both `image` and
`build`). An image named by its revision is that revision's, so nothing
stale under one tag is kept. Any other render is applied with `--build`,
as before. A host command (`organism place`) and a heart migration
(`[dna.migrations.<role>]`) are handed `HALE_DNA_REVISION` as well, so a
command can fetch a published build of itself instead of compiling its
source.

The manifest is read once, by graft; every reader after it reads the
roles the habitat allocated, as `allocation.json` keeps them.
