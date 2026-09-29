# The head and the face

The **head** is the edge API: the one surface people and services
reach. It knows who is calling (a session, a bearer, a socket peer),
maps them to the positions the record says they hold, and admits their
commands into the record with a receipt. It never performs work. The
operator who serves it owns it; it is built from source (`dna/api`) and
started for you by `dna/face/start.sh`. A head fails closed: with no
principal source it refuses to start, a caller it cannot name gets
nothing, and a command outside the caller's positions is refused. It
never holds the spine's database role, a model credential or the forge
token. The **face** is the browser shell over it, where the organism
shows itself to people.

| part | what it is | where it lives |
| --- | --- | --- |
| head | a project's API: reads over HTTP, commands on its api binding (a socket and an HTTP transport) | `dna/api` |
| the project head | the face's server: a registry of projects, receipts, a proxy to each project's head | `dna/api/project_service`, `dna/face/start.sh` |
| face | the browser shell | `dna/face` |
| surface | `hale dna ui`: a read-only page from the record | `dna/ui` |
| the Board and Reviews | the queue, the Review, its verdicts and signers | `hale dna board`, `hale dna review`, `dna/core/review.hl` |
| the forge mirror | Reviews as pull requests | `hale dna github sync`, `dna/host/forge_github.hl` |

## Start the head

From a checkout of hale, with a project made by `hale dna new`:

```sh
dna/face/start.sh /absolute/path/to/project
```

The launcher builds the project head and the project's API child in
temporary storage and prints the face's loopback URL. With no project
it starts detached, and the browser's Projects workspace creates,
initializes or attaches one. The head is source-built; no `hale dna`
verb starts it.

| port | what listens | flag |
| --- | --- | --- |
| 8792 | the project head, serving the face | `--port` |
| 8793 | the API child's reads | `--api-port` |
| 8794 | the stub OpenID provider (`dna/oidc`) | `--oidc-port` |
| 8795 | the API child's commands: its api binding's HTTP transport | `--commands-port` |

The API child says when each part is listening. Its commands port is
given to it (`HALE_DNA_COMMANDS_PORT`), never derived from the reads'
port, and once its binding holds that port it prints:

```text
hale dna api: commands http://127.0.0.1:<port>/
```

The launcher starts no body, database or broker; those are the
project head's operations, each a CLI verb it runs on request.

## Who you are

A head has exactly two ways in, and a caller is always a person, a
service, or nobody.

**On the socket**, a peer is the local account the kernel vouches for,
mapped to a person in the record's own config:

```sh
git config --local --add dna.unix.member "uid:1000=alice"
```

`hale dna new` and `hale dna init` map whoever runs them. The socket is
one per record, under `$XDG_RUNTIME_DIR/hale/dna/` (else the record's
`.hale/dna`), and the head names it in its capabilities. This is where `hale dna work` sends its
commands ([Legs, hands and voice](./legs.md)).

**Over HTTP**, every caller is an identity provider's subject. Name the
provider and map the subjects you trust:

```sh
git config dna.principal oidc
git config dna.oidc.issuer https://id.example.com
git config dna.oidc.client dna-acme
git config --add dna.oidc.member "<subject>=alice"
hale dna secret set OIDC_CLIENT_SECRET
```

The client secret goes into the vault as `oidc-client-<client>`
([The skin](./skin.md)). `dna.oidc.redirect` names the callback when it
is not the head's own `/auth/callback`. The head reads the issuer's
discovery document and signing keys once, at start, and does not start
without them. Every token is verified: ES256 under the issuer's keys,
the issuer, this client in the audience, unexpired. An issuer on the
loopback is anyone's who can bind its port, so the record pins its key
(`dna.oidc.key`), and the head trusts that key alone.

- **A person** signs in through the browser: the authorization-code
  flow, a random `state` and `nonce`, a callback bound to the browser
  that started it. The session is an `HttpOnly` cookie that lasts eight
  hours or until `/auth/logout`. An unmapped subject gets no session.
  A program acting for a person presents its ID token as
  `Authorization: Bearer <token>` instead.
- **A service**, a program with no person behind it, gets its own token
  from the issuer with the `client_credentials` grant and presents it
  as a bearer. Map it beside your members, with
  `git config --local --add dna.oidc.service "<subject>=<service>"`.
  It reads as `service:<service>`, gets no session, and sends no
  commands: a command is a person's, and a service holds no position.
  A subject is a person or a service, never both.

**Local mode is OIDC too.** `dna/face/start.sh` starts the stub
provider on the loopback and signs you in through it as the subject
`local-sub`, mapped to `$USER`; it configures a project it attaches for
that issuer, pins the stub's key, and refuses a project another issuer
serves.

**Trusted-local is a fixture's mode.** A head started with neither
`dna.principal = oidc` nor `HALE_DNA_TRUSTED_LOCAL=1` refuses to start:

```text
trusted-local is a fixture's mode (HALE_DNA_TRUSTED_LOCAL=1); a project serves its head under OIDC: `git config dna.principal oidc` with dna.oidc.issuer, dna.oidc.client and dna.oidc.member (dna/face/start.sh sets them up with the stub provider, dna/oidc)
```

The head speaks plain HTTP. On a network, put TLS in front of it.

## Commands and their receipts

Every command the record takes is a gated topic on the head's api
binding. The record answers who holds what, so the same gates apply on
the socket and over HTTP:

| gate | who passes | topics |
| --- | --- | --- |
| `owner` | a holder of `position:board` | `OrganizationPropose`, `TaskReassign`, `PersonRetire` |
| `reviewer` | a holder of `position:reviewer` | `ReviewVerdict` |
| `position` | a holder of any live position | `PracticePropose`, the `Attempt*` topics, `FrictionFile`, the `Knowledge*` changes |
| none | any person the head can name | `TaskCreate`, `CommandLookup`, `KnowledgeLookup` |

Over HTTP a command is a line of the wire, POSTed to the commands port
under the caller's bearer. A browser holds a session, not a bearer: the
head that serves the page relays its `POST …/commands` with the
session's token, after checking what a cookie needs (the exact
`Origin`, `X-Hale-Command: 1`, a JSON body).

A command is one act per request id. Its receipt names the command and
request ids, the operation, the principal (`principal_mode`,
`principal_name` and the positions the person holds),
the target, and a `state` with its `reason`. Sent again under the same
id, a command that landed is found by it and answered with the same
receipt; `CommandLookup` reads one back.

The project head keeps receipts of its own operations (create, init,
attach, sync, the local body, secrets, the model probe, and more), each
a CLI verb run detached, with its pid, exit code and log as files,
under a `command-<digest>` identity:

```text
recorded → admitted | refused → running → succeeded | failed | outcome_unknown
```

An identical retry replays the receipt; a changed one conflicts. A
verb that reaches beyond this machine and passes its deadline is
`outcome_unknown`, never `failed`. The head appends nothing to any
record itself.

## The Board's queue

`hale dna board` is what waits for the Board. A fresh project's:

```text
$ hale dna board
board: 16 review(s) need your verdict
  k:05cecb18af4e  ratify the design practice `design/software-delivery`: For an appendage or a product: process boundaries first (what runs, fail…
  k:076fd6de2420  ratify the design practice `design/standard-equipment`: Every part that supervises others is born with its architect: the positi…
…
secret: the forge's token is not in the vault (forge-token); `hale dna secret set FORGE_TOKEN` fills it
decide with `hale dna review <id> approve|revise|reject`; `hale dna review <id>` renders one
```

It lists the pending Reviews that need the Board, how many the Leader
is deciding inside its grant, handed Tasks waiting on a decision
(`tasks waiting:`), proposals, the last report, grant changes, a model
credential the body lacks, the secrets the vault lacks, and the latest
violations the organism absorbed. It reads the record, so it works in
any clone.

## Reviews

A Review owns one question: the candidate's digest, the authority or
the positions it requires, and the verdicts it received.

```sh
hale dna review                  # the pending Reviews
hale dna review <id>             # render one
hale dna review <id> --iris      # a change's semantic diff in iris's review view
```

The list groups the toolchain's seeded proposals under their headings
(`purpose`, `design`, `operating`, and a repository's `holes`,
`practices` and `holds`). A proposal renders as its question and what
it needs:

```text
$ hale dna review k:3870910f5de4
review k:3870910f5de4 [pending]: ratify the declared purpose?
  needs board · candidate sha256:3870910f5de44652ad0a9b91cedc50da02d81c3443181b6bed8fd50c0469bc8d
decide: hale dna review k:3870910f5de4 approve|revise|reject|abstain [--as <you>] [--comment <c>]
```

A change renders its **three views**, always together, because each
sees what the others cannot:

- **the source diff**, git's, base to candidate: the only view that
  sees handler behaviour, a changed literal, a field added and unread;
- **the semantic diff**, `hale model diff` from its receipt:
  declarations added, removed, renamed, moved; contracts and effect
  classes that moved; claims whose result flipped;
- **the evidence table**, one row per step the toolchain ran (`fmt`,
  `check`, `verify`, `test`, …), `yes` or `NO`, with the receipt of
  its output.

Above them the render names the mutation, its class, its author, the
disposition under the grant, and the magnitude. The render works
offline, from the record and its receipts. The kill test behind the
three views is in `dna/kill-test/WALKTHROUGH.md`.

### Verdicts

```sh
hale dna review <id> approve --comment "fine"
hale dna review <id> approve --digest <sha>     # name the candidate you looked at
hale dna review <id> reject --no-wait           # write it; the answer lands in the record
hale dna review design approve                  # every pending Review in a group, each its own verdict
```

The verdicts are `approve`, `revise`, `reject` and `abstain`. `--as`
names the reviewer (default: `$USER`), `--authority` the authority
claimed (default: `board`), `--comment` the reasoning, and `--digest`
the candidate you looked at (default: the one the Review pinned). The
verdict is a `review.verdict` row; a node relays it, and the Review
answers in the record. Unless you say `--no-wait`, the verb waits
for the answer:

```text
review <id> settled: …
review <id> signed by <who>; it awaits <positions>
review <id> refused the verdict: …
```

The Review admits a verdict only if:

1. **it names the pinned candidate** (else `digest mismatch: …`);
2. **the reviewer stands to decide it**: with signers, a holder of a
   required position; with none, a claimed authority that satisfies
   the required one (else `authority <a> does not satisfy <required>`);
3. **the reviewer did not author it** (`reviewer <who> authored the candidate`).

| authority | rank |
| --- | --- |
| `board` (also `maintainer`) | 4 |
| `leader` | 3 |
| `supervisor` | 2 |
| `reviewer` | 1 |

A claimed authority satisfies a requirement of its rank or below. `revise` and
`reject` settle the Review and leave the genome untouched; `abstain` is
recorded and leaves it open. An `approve` on a change is also its apply
([The heart and the body](./heart.md)).

### Route and signers

A change set goes to the positions that must sign it, read from the
graph's `reviews` edges, never guessed:

```sh
hale dna route main.hl
hale dna route --diff main..HEAD
hale dna route --json main.hl
```

It lists who signs (each position with its holders and why), the gates
the change is judged against, what no position reviews, and what is
left to the fallback. The graph is memory's, so without memory it
says so:

```text
$ hale dna route main.hl
hale dna: route: the graph is memory's, and no memory is named (HALE_DNA_MEMORY_DSN_HEAD); `hale dna memory migrate` prints it
```

A Review takes its signers and gates from the same computation when it
opens, and records them as `review.routed`. With signers, approval
settles only once every required position has approved and every gate
has a passing run at the candidate, as the forge reports it
(`gate.observed`); one rejection or revision from a required position
settles it.
With no signers (no memory, or no position the route names is held),
the Review keeps its required authority, which the render shows as the
fallback. The Leader never decides a routed Review. A verdict cites no
gate run: `--evidence` is refused.

## The face and the surface

**The face** (`dna/face`) is the browser shell the project head serves:
the Projects workspace, where you create, initialize and attach
projects and run their operations, and, for an attached project, its
application controls, the organization, workflow definitions,
Knowledge, practices and Reviews. It does not poll: the head tells it when to read
again over an event stream (`GET /api/hale/v1/head/events`), when a
receipt is journaled, a run or child starts or ends, or the organism
lands a row.

**`hale dna ui [project] [--port N]`** serves a page from the record
alone (port 8790 by default). Every request runs one offline verb in
the project root and returns what it printed: the status, the Board's
queue, the pending Reviews and one Review's three views, the fleet,
pressure, and the history. It reads, and takes no command: a POST is
refused, because the record's commands are the head's gated topics.
It serves under `dna.principal = oidc` (with `dna.oidc.issuer`,
`dna.oidc.client` and `dna.oidc.redirect` set), a session in front of
every page, and otherwise refuses to start, as any head does.

## The GitHub mirror

GitHub is a projection of the record, and the record wins:

```sh
git config dna.github acme/chat
git config dna.github.board alice,bob     # whose GitHub reviews carry the Board's authority
hale dna secret set FORGE_TOKEN
hale dna github sync
```

`hale dna github sync` opens a pull request for every pending change
Review (the candidate pushed to `dna/<id>`, the three views as the
body, `github.pr` in the record), reads every GitHub review on it back
as a `review.verdict` in the reviewer's login (`board` when
`dna.github.board` lists the login, `reviewer` otherwise, each once),
records the gate runs it finds at the candidate (`gate.observed`), and,
once a change is applied, comments the settlement (`github.commented`)
and pushes the genome on approval. A host under `hale dna dev` or
`hale dna run` does the same on its tick. `gh` runs under the vault's
forge token alone; with none, nothing runs, and `gh`'s own login is
never used in its place.

## How it breaks

- **The head refuses to start**: no principal source (the trusted-local
  message above), an issuer that is not https or loopback, an issuer
  whose keys cannot be read, or a loopback issuer with no pinned key.
- **`not a member: subject … maps to no one`**: map the subject with
  `dna.oidc.member`.
- **A command refused `forbidden`**: the caller holds no position the
  topic's gate names, or is a service.
- **`commands_unavailable`**: the head was given no commands port.
- **A verdict refused**: the reason is one of the three checks above;
  `hale dna review <id>` shows it under `refused a verdict:`.
- **`no forge token`**: `hale dna secret set FORGE_TOKEN`.
