# Edge API Server, Web UI, and Reviews (The Head and the Face)

The **head** (`dna/api`) is the edge API server for a DNA project. It authenticates callers (via OIDC sessions, bearer tokens, or Unix domain sockets), resolves their held positions from the record, and admits commands (`HeadCommands`) into the Postgres record with idempotent receipts. The **face** (`dna/face`) is the browser UI shell served in front of the head. Neither the head nor the face executes tasks or holds the spine's database role, model API keys, or GitHub forge tokens.

```sh
# Start the project head, standard browser face (port 8792), and local OIDC stub
hale dna head /absolute/path/to/project

# Inspect the Board queue and render or decide a pending Review from the CLI
hale dna board
hale dna review <id>
hale dna review <id> approve --comment "verified semantic diff and test receipts"
```

## Architecture Overview

| Component | Engineering Role | Source / Entrypoint |
| --- | --- | --- |
| **Head** (`dna/api`) | Project edge API: serves HTTP read queries and exposes the `HeadCommands` mutation surface over a Unix socket and HTTP | `dna/api` |
| **Project Head** | Multi-project server: manages project registry, detached CLI operation receipts, and proxies to each attached project's API child | `dna/api/project_service`, `hale dna head`, `dna/face/start.sh` |
| **Standard Face** | Browser UI shell serving `/index`, `/app/`, and the Projects workspace in front of the head | `dna/face` |
| **Read-Only UI** (`hale dna ui`) | Standalone read-only HTTP server rendering offline CLI projections on port `8790` | `dna/ui` |
| **Board & Reviews** | Governance queue, 3-view Review rendering, signer routing, and verdict settlement | `hale dna board`, `hale dna review`, `hale dna route`, `dna/core/review.hl` |
| **Forge Mirror** | Reviews mirrored as pull requests at the organism's allocated forge, and their verdicts read back | `hale dna forge sync`, `dna/core/forgejo.hl`, `dna/host/forge_github.hl` |

---

## Starting the Project Head (`hale dna head`)

For any project created or attached with `hale habitat graft`, start the head and browser face with:

```sh
hale dna head /absolute/path/to/project
```

On first run, `hale dna head` compiles the project head, standard face, project API child, stub OIDC provider (`dna/oidc`), and reflexes into the toolchain cache (subsequent starts launch immediately until the toolchain version changes) and prints the face's loopback URL. If invoked without a project path, the server starts detached so you can attach or manage projects from the browser's Projects workspace.

When developing from a `hale` compiler checkout, `dna/face/start.sh /absolute/path/to/project` accepts the exact same flags (`--help` lists all options) and builds directly from the working tree into temporary storage on each start.

### Default Ports and Dynamic Port Binding

| Default Port | Service | CLI Flag |
| --- | --- | --- |
| `8792` | Project head serving the standard browser face | `--port` |
| `8793` | Project API child (HTTP read endpoints) | `--api-port` |
| `8794` | Local stub OpenID Connect provider (`dna/oidc`) | `--oidc-port` |
| `8795` | Project API child (`HeadCommands` HTTP mutation surface) | `--commands-port` |
| `8796` | Internal loopback port where the head listens behind the standard face | `--head-port` |

When `dna/api/project_service` is started with an API port of `0` and no `HALE_DNA_COMMANDS_PORT` (such as in test fixtures), it passes port `0` for both sockets to the API child. The child binds ephemeral OS ports and reports both back via `HALE_DNA_API_PORTS` (preventing port-race collisions), printing:

```text
hale dna api: commands http://127.0.0.1:<port>/
```

The API child serves the same surface over MCP too, for an agent pairing with a person, on an ephemeral port it always picks itself; it reports it as `mcp=<port>` in `HALE_DNA_API_PORTS` and prints:

```text
hale dna api: mcp mcp://127.0.0.1:<port>
```

### Database Role and Memory Requirements

The head reads the record and Knowledge from Postgres under the least-privilege **head role** (`HALE_DNA_MEMORY_DSN_HEAD`). `hale dna head` derives this DSN from an active, migrated memory allocation and passes it exclusively to the head process without printing it; an explicit `HALE_DNA_MEMORY_DSN_HEAD` in the environment takes precedence.

The head never starts databases, brokers, or the organism body, and never runs schema migrations. If the project's memory allocation is down or unmigrated, the head refuses to start and directs you to run `hale habitat migrate <project>`.

---

## The Habitat Face and Operator UI

Because the local habitat is itself a DNA organism, `hale habitat up` starts the standard face over the habitat's own head alongside its body. `hale habitat status` prints the URL on the `face` line (`hale habitat down` stops it):

```sh
$ hale habitat status
...
face    http://127.0.0.1:8864/ (up, compose service face)
```

- **Habitat Index (`/index`):** Lists every organism grafted from the habitat under **Organisms**, showing its name, state (`up` while its face responds, `grafted` otherwise), face URL, and head URL (each organism receives a dedicated block of loopback ports stored in `dna.head.url` at graft time).
- **Infrastructure Surface Calls:** The habitat head exposes `Commands::hosts` (registered machines and their capabilities), `Commands::allocations` (every organism role's placement provider, host, and live state: `up`, `degraded`, or `down`), and `Commands::policy` (role-to-provider mapping). The face queries these endpoints via `POST /api/hale/v1/head/surface/call/Commands::hosts` (and sibling rows) with `{"limit": 50}`.
- **Habitat Operator UI (`/app/`):** Served directly from `dna/habitat/ui/` in the habitat's genome using a generated ES-module client (`hale api client --lang js`, committed at `dna/habitat/ui/head-client.js`) with zero build step. Unauthenticated visitors are redirected to the OIDC provider before the page is served. Styling is defined in `dna/habitat/ui/operator.css` (brass indicates a boundary crossing; color is reserved strictly for operational state). The page is four screens switched by the hash: the habitat (its name and state, and the web UIs across its organisms from `Commands::uis`: each organism's face and each UI its manifest declares under `[dna.ui.<name>]`, with its link and how to sign in), **hardware** (hosts, their offers and host keys, add a host and copy the key line, probe, remove: a host with allocations shows its removal's plan, `Commands::plan_remove_host`, and a field for the host's name, which `Commands::remove_host` checks), **infrastructure** (allocations with a host picker, `apply` and its pulse, the confirm step for a move that starts a memory empty) and **organisms** (the organisms with purpose and pulse, the saplings, a `graft` per sapling that follows the graft's log, and `remove` on each organism: its plan from `Commands::plan_remove_organism` and a field for its name). A refusal is shown as the row said it.

---

## Authentication and Principal Mapping

A head fails closed: it refuses to start without a configured principal source, rejects unmapped callers, and speaks plain HTTP (terminate TLS at a reverse proxy when exposing over a network). Callers authenticate through one of two channels:

### 1. Local Unix Domain Socket

Commands sent by `hale dna work` ([Legs, hands and voice](./legs.md)) travel over a per-record Unix domain socket located under `$XDG_RUNTIME_DIR/hale/dna/` (or `.hale/dna/` in the checkout), advertised in the head's capabilities. The OS kernel vouches for the peer's UID, which the head maps to a person via local Git config (written automatically for the current user by `hale habitat graft`):

```sh
git config --local --add dna.unix.member "uid:1000=alice"
```

### 2. HTTP and OpenID Connect (OIDC)

Over HTTP, every caller is authenticated against an OIDC identity provider:

```sh
git config dna.principal oidc
git config dna.oidc.issuer https://id.example.com
git config dna.oidc.client dna-acme
git config --add dna.oidc.member "<subject>=alice"
hale dna secret set OIDC_CLIENT_SECRET
```

- **Vault & Discovery:** `hale dna secret set OIDC_CLIENT_SECRET` stores the client secret in the vault slot `oidc-client-<client>` ([The skin](./skin.md)). Set `dna.oidc.redirect` if the callback URL differs from the head's `/auth/callback`. At startup, the head fetches the issuer's OIDC discovery document and JWKS signing keys (and refuses to start if they are unreachable). Every token is verified for `ES256` signature, issuer, audience (`dna.oidc.client`), and expiration. For loopback issuers (`127.0.0.1` / `localhost`), the record must pin the issuer's public key in `dna.oidc.key`, and the head trusts only that key. Unauthenticated clients can query `/.well-known/hale-dna` to discover the head's issuer and client ID.
- **Human Principals (`dna.oidc.member`):**
  - *Browser sessions:* Authenticate via the OIDC authorization-code flow with random `state` and `nonce` bound to the initiating browser. Mapped subjects receive an 8-hour `HttpOnly` session cookie (cleared by `/auth/logout`); unmapped subjects receive no session.
  - *CLI / Leg bearer tokens:* Present `Authorization: Bearer <id-token>`. Against the local stub issuer, `hale dna work login` mints this token; against an external issuer, users sign in at `/auth/login`.
  - *Lease-gated reads:* Context packages (`/dna/context`, used by `hale dna work brief`) can only be read by the person whose bearer token matches the active lease holder on that Work's attempt.
- **Service Principals (`dna.oidc.service`):** Automated services without a human operator obtain tokens via the OIDC `client_credentials` grant and map their subject with:
  ```sh
  git config --local --add dna.oidc.service "<subject>=<service>"
  ```
  Service principals authenticate as `service:<service>`, receive no browser session, hold no positions, and can only execute read queries—never commands. A subject cannot be mapped as both a person and a service.
- **Local Stub OIDC Mode:** `hale dna head` and `dna/face/start.sh` automatically launch the local stub OIDC provider (`dna/oidc`), sign you in as subject `local-sub` mapped to `$USER`, pin the stub's key in `dna.oidc.key`, and refuse to attach projects configured for a different issuer.
- **Fixture Mode (`HALE_DNA_TRUSTED_LOCAL=1`):** Starting a head without `dna.principal = oidc` or `HALE_DNA_TRUSTED_LOCAL=1` fails immediately:
  ```text
  trusted-local is a fixture's mode (HALE_DNA_TRUSTED_LOCAL=1); a project serves its head under OIDC: `git config dna.principal oidc` with dna.oidc.issuer, dna.oidc.client and dna.oidc.member (dna/face/start.sh sets them up with the stub provider, dna/oidc)
  ```

---

## `HeadCommands` Surface, Authorization Gates, and Receipts

Every mutation accepted by the record is a member of the `HeadCommands` API surface. Both the CLI and the browser UI call this exact same generated surface (`hale api client --surface HeadCommands`; the Hale client for legs is committed in `dna/core/legs/head_commands`).

### Authorization Gates

Authorization gates are evaluated identically over the Unix socket and HTTP based on the positions held by the authenticated person in the record:

| Gate | Required Seat | `HeadCommands` Operations |
| --- | --- | --- |
| `owner` | Holder of `position:board` | `propose_organization`, `reassign_task`, `retire_person` |
| `reviewer` | Holder of `position:reviewer` | `verdict` |
| `position` | Holder of any active position | `propose`, attempt lifecycle (`claim`, `outcome`, `renew`, `release_attempt`, `ask_allowance`), `file_friction`, `*_knowledge` mutations, org structure (`open_position`, `fill`, `apply_batch`) |
| `none` | Any authenticated person | `create_task`, `lookup`, `lookup_knowledge`, `preview_batch` |

Over HTTP, clients invoke `POST /call/<member>` on the commands port (`8795`) with a bearer token. For browser sessions, the face server validates CSRF preconditions (exact `Origin` match, `X-Hale-Command: 1` header, and JSON content type) on `POST …/commands` and relays the request using the session's token.

### Structural Mutations and Batch Previews (`preview_batch` / `apply_batch`)

UI and API calls to `open_position`, `fill`, and `apply_batch` write the exact same rows—and enforce the exact same refusal rules—as `hale dna position open`, `hale dna fill`, and `hale dna apply` (`vacate` is submitted as a one-step batch: `vacate <position> <holder>`). Every structural mutation requires `record_head` (the record digest it was prepared against) and fails with `stale_subject` if the record has advanced.

Before applying a batch, UIs call `preview_batch` to compute a read-only diff against the current record head:

```text
preview_batch { steps }            → { ok, why, head, before, after, added, removed, unchanged, pending }
apply_batch { request_id, record_head: head, steps, pack: "" }
                                   → receipt.structure { state: requested, request_id, review_id, … }
```

The returned receipt's `structure.state` begins as `requested` and transitions to `proposed` (with `review_id`) or `refused` (with `why`) once the organization processes it; submitting identical steps while a Review is still open returns `already_proposed` with the existing `review_id`.

### Command and Operation Receipts

- **Surface Command Receipts:** Keyed by `request_id`. Each receipt records the command ID, `request_id`, operation name, principal metadata (`principal_mode`, `principal_name`, and held positions), target, `state`, and `reason`. Retrying a command with the same `request_id` returns the stored receipt (`lookup` fetches a receipt by ID).
- **Detached Project Head Operations:** When the project head runs detached CLI operations (such as grafting, attaching, syncing, starting the local body, setting secrets, or probing models), it tracks each execution by its `command-<digest>` identity alongside its PID, exit code, and log file through the state machine:
  ```text
  recorded → admitted | refused → running → succeeded | failed | outcome_unknown
  ```
  Retrying an identical operation replays the receipt; changing arguments with the same ID conflicts. Any operation that crosses the network and exceeds its deadline is marked `outcome_unknown` (never assumed `failed`).

---

## The Board Queue (`hale dna board`)

`hale dna board` reads the record offline from any clone and summarizes every item awaiting Board action or operator intervention:

```text
$ hale dna board
board: 16 review(s) need your verdict
  k:05cecb18af4e  ratify the design practice `design/software-delivery`: For an appendage or a product: process boundaries first (what runs, fail…
  k:076fd6de2420  ratify the design practice `design/standard-equipment`: Every part that supervises others is born with its architect: the positi…
…
secret: the forge's token is not in the vault (forge-token); `hale dna secret set FORGE_TOKEN` fills it
decide with `hale dna review <id> approve|revise|reject`; `hale dna review <id>` renders one
```

The output includes:
- Pending Reviews requiring `board` authority (plus a count of Reviews currently delegated to the Leader within its autonomy grant).
- Human-assigned Tasks awaiting completion or a third-party decision (`tasks waiting:`).
- Pending organizational proposals, grant changes, and the most recent filed report.
- Missing vault credentials (`FORGE_TOKEN`, model API keys) and the latest absorbed runtime violations (`violation.recorded`).

---

## Reviews, Verdicts, and Signer Routing

Every Review (`dna/core/review.hl`) governs a single candidate digest, tracking the required authority or signer positions and all submitted verdicts.

```sh
hale dna review                  # list all pending Reviews grouped by family
hale dna review <id>             # render a Review offline (question, 3 views, and status)
hale dna review <id> --iris      # inspect a code change's semantic diff in the Iris review view
```

`hale dna review` groups pending Reviews by family in proposal order: `purpose`, `design`, `operating`, `using`, `library`, `holes` (where each proposed position is immediately followed by its `mandate/<position>` Review so `hale dna review holes approve` ratifies and binds them in sequence), `practices`, `holds`, `positions`, `changes`, and `ingest`.

### The Three Review Views

When rendering a code mutation Review, `hale dna review <id>` displays the mutation metadata (class, author, grant disposition, and magnitude vector) followed by **three mandatory views** computed offline from the record and its receipts:
1. **Source Diff (`git diff`):** Base-to-candidate line diff—the only view that exposes handler logic tweaks, literal changes, or unused struct fields.
2. **Semantic Diff (`hale model diff`):** Structural summary of added, removed, renamed, or moved declarations, contract/effect class shifts, and constitutional claims whose verification status changed.
3. **Evidence Table:** Pass/fail (`yes` / `NO`) table for every verification step executed by the toolchain (`fmt`, `check`, `verify`, `test`, `fleet`, `replay`, `rollback`), linked to its output receipt digest.

### Submitting Verdicts

```sh
hale dna review <id> approve --comment "fine"
hale dna review <id> approve --digest <sha>     # explicitly pin the candidate digest you inspected
hale dna review <id> reject --no-wait           # write verdict row and return without waiting for settlement
hale dna review design approve                  # approve every pending Review in the `design` group sequentially
```

| Flag / Argument | Description |
| --- | --- |
| `approve\|revise\|reject\|abstain` | Verdict decision (`approve` on a code change also triggers genome application; `revise` and `reject` close the Review without touching the genome; `abstain` records a note and leaves the Review open). |
| `--as <reviewer>` | Reviewer identity (defaults to `$USER`). |
| `--authority <a>` | Claimed authority level (defaults to `board`). Ranks: `board` / `maintainer` (`4`) > `leader` (`3`) > `supervisor` (`2`) > `reviewer` (`1`). |
| `--comment <c>` | Justification recorded in `review.reasoned`. |
| `--digest <sha>` | Expected candidate digest (defaults to the Review's pinned digest). |
| `--no-wait` | Do not block waiting for the organization to settle the `review.verdict` row. |

A Review admits a verdict only if all three conditions hold:
1. **Candidate digest match:** `--digest` matches the Review's pinned candidate (otherwise refused with `digest mismatch: …`).
2. **Sufficient authority or signer seat:** If the Review is routed to specific signer positions, the reviewer must hold one of those required positions; otherwise, the reviewer's claimed `--authority` rank must be greater than or equal to the Review's required authority (`authority <a> does not satisfy <required>`).
3. **Author separation (two-person rule):** The reviewer must not be the author of the candidate (`reviewer <who> authored the candidate`). The sole exception is a Board-level governance proposal (filling or opening a position, or a graph proposal), where the proposer may also ratify as the Board ([Positions and `fill`](./shaping.md#positions-and-fill)).

### Signer Routing (`hale dna route`)

For code changes, required signers and CI gates are computed deterministically from `reviews` edges in the Postgres memory graph:

```sh
hale dna route main.hl
hale dna route --diff main..HEAD
hale dna route --json main.hl
```

`hale dna route` prints every position that must sign the change (with its current holders and the graph edge that triggered it), the required verification gates, any unreviewed paths, and the fallback authority. (Because routing queries the memory graph, running `hale dna route` without `HALE_DNA_MEMORY_DSN_HEAD` prints an error directing you to run `hale habitat migrate <organism>`.)

When a Review opens, it records this routing calculation as a `review.routed` row:
- **With active signers:** The Review settles as approved only after **every** required position submits `approve` and **every** required gate records a passing run at the candidate digest (`gate.observed` from the forge). A single `reject` or `revise` from any required position settles the Review immediately. The Leader never decides a routed Review. (Passing `--evidence` on a verdict is refused; gate runs are observed directly from the forge.)
- **Without signers** (no memory configured, or required positions are currently vacant): The Review falls back to its required authority rank (`board`, `leader`, etc.).

---

## Browser Surfaces: Standard Face, `/index`, Named Faces, and `hale dna ui`

- **Standard Face (`dna/face` on port `8792`):** Serves the interactive Projects workspace and project controls, updating reactively via Server-Sent Events (`GET /api/hale/v1/head/events`) whenever receipts, runs, or record rows land. Always open the face's port (`8792`), not the internal head port (`8796`, which serves only API surfaces and returns `404` for `/app.js` or `/app/`).
- **Project Index (`/index`):** A zero-JavaScript HTML summary generated fresh on each request via `Commands::index` (`HeadCommands`), displaying the project's purpose, parts, position holders, attached codebases, watched upstreams (with latest readings and timestamps), contracts, and external knowledge links. Under OIDC, `/index` requires an authenticated session.
- **Buildless Application UI (`/app/`):** If an attached repository contains a static `ui/` directory (with no `package.json` or build manifest) that is referenced by `README.md` or mounted in `compose.yaml`, the standard face serves `ui/index.html` at `/app/` and confines all asset reads strictly inside `ui/`.
- **Named Faces (`dna::Face`):** An organism's main locus can declare additional custom UI applications as parameters of type `dna::Face`:
  ```hale,fragment
  admin: dna::Face = dna::Face {
      app: "faces/admin",
      roles: "operator",
      consumes: "Orders",
      port: 8801,
  };
  ```
  - `app`: Path to the face's seed directory.
  - `roles`: Space-separated list of roles the face acts under.
  - `consumes`: Space-separated list of API surfaces the face calls.
  - `port`: Loopback port the face listens on.
  `hale check` verifies that every consumed surface and role is declared by the program and that ports do not collide. `hale dna dev` and `hale dna run` compile each named face and launch `<binary> <port>` with `HALE_DNA_FACE` and `HALE_DNA_FACE_PORT` (with zero organism credentials). The standard face links to each named face on `/index`.
- **Standalone Read-Only Server (`hale dna ui [project] [--port N]`):** Serves a read-only HTML projection of the record on port `8790` by executing offline CLI queries per request (status, Board queue, Reviews and 3-view diffs, fleet, pressure, and history). It rejects all `POST` requests and requires `dna.principal = oidc` (`dna.oidc.issuer`, `dna.oidc.client`, `dna.oidc.redirect`) to start.

---

## The forge mirror (`hale dna forge sync`)

The forge is a gate, not a store. Code goes out through it as branches and verdicts come back through it; the organism reads nothing from it that it did not already decide. A Review lives in the record, its candidate is a branch in git, and the pull request is a mirror of it. The record wins.

Which forge stands in the gate is an allocation, a role the habitat allocates as it allocates memory. On your machine it is the habitat's own Forgejo: `hale habitat up` brings it up with memory and the nerves, and a graft gives the organism a Forgejo user and a repository there, its token in the vault (`forgejo-<id>`). Nothing to set up:

```sh
hale habitat graft chat
hale dna forge sync          # the branch goes out; pending Reviews become pull requests
```

GitHub is the allocation when the Board names it:

```sh
git remote add origin git@github.com:acme/chat.git
hale habitat forge chat github --repo acme/chat --key-vault forge-token
git config dna.github.board alice,bob     # whose GitHub reviews carry the Board's authority
hale dna secret set FORGE_TOKEN
hale dna forge sync
```

Moving a project from the local forge to GitHub is exactly that: change the remote, change the allocation, sync. A Review still open reappears as a pull request on GitHub; nothing migrates, and the Forgejo pull request is left as it was. `none` (a bare remote, Reviews decided in the organism's own face or CLI) is there when there is no forge at all, and `hale dna forge sync` then says `forge: no forge allocated`.

Against whichever forge is allocated, `hale dna forge sync` (or the host, on each tick under `hale dna dev` / `hale dna run`):
1. Pushes each pending change Review's candidate commit to branch `dna/<id>`, opens a pull request with the three Review views as its description, and records `github.pr` in the record.
2. Reads every review on it back as a `review.verdict` in the reviewer's login (`board` when `dna.github.board` lists the login, `reviewer` otherwise, each once).
3. Records the gate runs it finds at the candidate as `gate.observed` rows.
4. Once a change is applied, comments the settlement (`github.commented`) and pushes the genome on approval.

A leg's review at an attached codebase comes back as `task.delivered` when merged and `task.refused` when closed, the comments on it as receipts. Each forge acts as its allocation's token alone, the vault slot it names. A practice never names a forge: "open a review" is the verb, and the allocation answers it.

> [!NOTE]
> `gh` executes strictly using the `forge-token` slot from the vault (`hale dna secret set FORGE_TOKEN`). If the vault slot is empty, `forge sync` runs nothing and never falls back to a developer's personal `gh auth` session.

---

## Troubleshooting the Head, Face, and Reviews

| Diagnostic / Symptom | Cause & Resolution |
| --- | --- |
| **Head refuses to start** (`trusted-local is a fixture's mode…` or `the issuer … is not usable`) | Missing `dna.principal = oidc`, non-HTTPS external issuer, unreachable JWKS keys, or a loopback issuer without a pinned `dna.oidc.key`. Configure OIDC or start via `hale dna head` / `dna/face/start.sh`. |
| `not a member: subject … maps to no one` | The authenticated OIDC `sub` claim is not mapped in git config. Run `git config --add dna.oidc.member "<subject>=<person>"`. |
| **Command refused with `forbidden`** | The caller does not hold a position required by the `HeadCommands` gate (`owner`, `reviewer`, `position`), or the caller is a `service:` principal. |
| `commands_unavailable` | The head process was started without a commands port (`--commands-port` / `HALE_DNA_COMMANDS_PORT`). |
| **Read or command answered `503` as the head stops** (`head_api_unavailable`, `commands_unavailable`) | A head stopped while waiting on its API child answers in-flight requests immediately with `503` and exits rather than holding its shutdown; retry once the replacement head is up. |
| **Verdict refused** (`digest mismatch`, `authority … does not satisfy …`, `reviewer … authored the candidate`) | Inspect `hale dna review <id>` under `refused a verdict:`. Re-render to check the current candidate digest, claim an authority rank that satisfies the Review, or have a second person review your code change. |
| `no forge token` | Store a GitHub token in the vault with `hale dna secret set FORGE_TOKEN`. |
