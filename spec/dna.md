# DNA

DNA is the part of a Hale application that governs how the application
changes. It ships as a library inside the toolchain (`vendor/dna`, the
`dna/core` seed of the hale repository) and as the `hale dna` commands.
This file specifies what the library and the commands promise; the
design is GH #521 and its successor #566; `docs/src/dna/` is the guide.

## The record

The Journal is a git branch, `refs/dna/journal`, in the governed
repository:

- **One commit per event.** The commit's tree holds `journal.jsonl`,
  every event so far, one JSON object per line: `seq`, `kind`,
  `entity`, `body`, `author`. The commit's subject is `<kind> <entity>`.
- **The commit DAG is the chain.** An event's digest is its commit;
  its `prev` is the parent; the first event has no parent. Integrity
  is git's: the ref resolves, and its commit count is the event count.
- **Append is compare-and-swap on the ref.** A writer builds the next
  commit on the head it read and updates the ref with that head as the
  expected old value. A writer that lost the race reloads and, when it
  was appending at the tail, re-appends at the new tail; `seq` is the
  position in the record, never a promise made before the append. A
  lost race is not a failure: the append goes round with the new tail
  until it lands, within a bound only a wedged repository reaches.
- **Authorship is git's.** The organism's own events carry its
  configured author; a human's facts (a verdict, an intent through the
  CLI, a host's crash accounting) carry the git identity of whoever
  ran the command. Commit signing, where the repository requires it,
  applies unchanged.
- **Receipts** are blobs under `refs/dna/receipts/<sha256>`, filed by
  the digest of their content: a verification step's output, a diff
  document. Events name receipts by digest. A receipt has a data class
  (GH #606): a `public` or `internal` body is such a blob; a `customer`
  or `confidential` body (`protected_class`) never is. The organism
  hands it to the knowledge service (`Dna.file_evidence(text, class,
  by)`, through `ReceiptVault`), which keeps it in the record's own
  schema and appends `receipt.classified <digest> {class, by, store}`.
  `PqProtected` encrypts it at rest with pgcrypto (OpenPGP, AES-256)
  under `HALE_DNA_RECEIPT_KEY` (sixteen characters at least), read into
  a sealed locus and sent to the database only as a query parameter, so
  an operator must not log statement parameters; `MemProtected` keeps it
  for the life of the process. With no service to keep it, the body is
  withheld: `receipt.withheld <digest> {class, by, why}`, and nothing
  holds it. The record keeps the digest and the class either way, so
  `sync` never carries a protected body into a clone. Reading one is an
  act in the reader's name for a purpose: the service answers `GET
  /receipt/<digest>?as=<reader>&purpose=<purpose>` only when a
  `receipt.disclosed <digest> {recipient, purpose, by}` row names both,
  and appends `receipt.read` on an answer and `receipt.read_refused` on a
  refusal. `Dna.evidence_class(digest)` is the class a request carries
  when it puts the body in a prompt, so a hosted model refuses it. This
  is the third trust profile #606 names: the readers of a clone hold
  digests and classes, never protected bodies. Under local trust a
  reader's name is attribution, as `--as` is; a verified principal is
  #612's. **Retention (part 2).** `receipt.held <digest> {by, why}`
  stands until `receipt.hold_released`, and refuses redaction while it
  stands. `Dna.redact_evidence(digest, by, why, policy)` (`hale dna
  receipt redact <digest> --why --policy`) removes the body and appends
  `receipt.redacted <digest> {by, why, policy, class, store}`: a git
  receipt's ref is deleted (`Receipts.erase`), a protected body is erased
  by the knowledge service (`POST /receipt/<digest>/erase`, which writes
  the row), a withheld one had no body. The record keeps the digest, so
  provenance survives and a reader learns the body is gone — the service
  answers a read with 410 `redacted by … under …`, and `hale dna history`
  says so under a redacted prompt. Every `sync` deletes redacted
  receipts' refs from the clone and the remote, so a clone that fetched
  one drops it at its next sync; the blob stays in each object store
  until git collects it (`git gc --prune=now`), and a copy disclosed or
  cloned outside the record cannot be recalled. The knowledge tail
  retires an idea whose receipt was redacted instead of stopping.
- **Leases** are blobs under `refs/dna/lease/<key>` (`:` in a key
  becomes `/`), `holder`, `token`, `expires`, `present` on four lines,
  compare-and-swapped on the ref. Tokens are monotonic per key.
- **Effects are requested before they run, and the claim is exclusive
  (GH #604 rule 3).** `effect.requested <key>` is appended before a
  dispatch and `effect.result <key>` once after; a retry finds the
  request and does not dispatch again. Two writers may both append a
  request for one key; only the writer whose row is the first request
  dispatches, the other's row stays as a contended claim and it does
  not act. A request with no result when the organism restarts is
  resolved by evidence where evidence exists (an `apply:<candidate>`
  whose genome is at the candidate is `ok`; a `worktree.open` with no
  worktree is `failed`) and marked `unknown` where it does not; the
  gate refuses an unknown effect until a person resolves it —
  `hale dna effect resolve <key> --outcome ok|failed`, one row in
  their name, the last result for the key being its status.
- **Sync.** `hale dna sync` (and the host, every tick) fetches the
  remote's record into `refs/dna/remote/journal`, reconciles, and
  pushes. Local ahead: push. Remote ahead: fast-forward. Diverged: the
  local-only events are re-appended on top of the remote's head, bodies
  and authors unchanged, `seq` their new position, then pushed; a push
  the remote refuses is fetched and reconciled again. **The reconciled
  chain is built beside the ref and swapped in with one
  compare-and-swap**: the ref never points at a record missing a row
  it held a moment before, so a reader's position only grows, a writer
  that reloads after a lost race finds its own rows, and a clone that
  appended while the chain was built is not overwritten — the swap
  fails and the reconcile goes round. A local-only event with no row
  refuses the reconcile rather than being skipped. A fact's identity
  in the record is its content and its place among rows of the same
  content, never its `seq` (a reconcile renumbers, and a node raising
  one concern three times writes three identical rows, each a fact):
  a host relays and handles by that identity, so a renumbered fact is
  not relayed twice and a repeated one is not relayed once. **A remote with
  no record yet is the local one's push**, not "up to date": the first
  sync after an ordinary `git push origin main` carries the record, so
  no one has to push `refs/dna/*` by hand for a second clone to have
  the organization's history. Receipts travel
  by refspec both ways. The remote is `dna.remote` in git config, or
  `origin`. A plain clone has no record until it syncs.
- **An apply expresses the candidate, tree and all.** An approval
  applies exactly the reviewed candidate, or nothing: the candidate's
  worktree head is the pinned digest, the genome's head is the base the
  review was against, and the genome has nothing uncommitted — tracked
  changes anywhere, and **every input of the candidate's build that
  the tree does not hold**. The graph is the candidate's, read by the
  compiler in the candidate's worktree (`hale inputs <seed>`: the
  seed's `.hl` files and those of every directory they import,
  transitively; the `hale.toml` of each such directory and every C
  source it declares under `[ffi]`), because a reviewed change that
  adds an import makes files an input the moment it lands; any file
  of those kinds the genome holds untracked in one of the graph's
  directories, or that the graph names outright, is dirt whether git
  ignores it or quotes it. What no build reads — a binary, a log,
  another program's source — is not dirt. A graph that cannot be
  inspected is dirt: an inspection that failed authorises nothing. A
  dirty genome is `mutation.refused` before any effect row exists,
  the work untouched; the gateway checks it once more around the git
  call and reads the head back, so an apply that reports success is
  the candidate. **Approving again** an approved and unapplied
  candidate re-runs the whole gate (`mutation.apply_retried`, then
  `review.settled` again so a waiting `hale dna review` hears it):
  the Review readmits only the same approval in full — digest,
  authority, independence, the word `approve` — and refuses any other
  verdict; a Review approved and never applied is rehydrated settled,
  so the road survives a restart.
- **A person's job (GH #596 W; GH #604 rules 4 and 5).** A plan of kind
  `person` hands the Task on: the Work is done as far as the organism
  is concerned and the record keeps `task.handed <task>` — `work`,
  `assignee` (the person the plan named), `by`, `narrative`; nothing
  is mutated. The person reports it done with `hale dna task done <id>
  [--as <who>] [--note …]`, a `task.done` row in their name and nothing
  else. **Completion is checked at admission:** a handed Task with an
  assignee is closed only in the assignee's name; anyone else is
  refused and told to reassign. `hale dna task reassign <id> --to <who>`
  appends `task.reassigned` (`from`, `to`, `by`) and the Task stays
  handed. `hale dna retire <who> [--to <successor>]` stops new work
  reaching a person: every handed Task they hold is transferred as its
  own `task.reassigned` row and `person.retired <who>` records it;
  refused while they hold work and no successor is named — pending
  work is accounted for, never dropped. Refused for a Task that is not
  handed: one the organism is working, or has settled, is not a
  person's to close.
- **Cross-record handoff (GH #615).** A replica of one record is inside
  the horizon: sync carries all of it, to equally trusted readers. A
  *handoff* crosses a horizon into a separate record, and a connection is
  its only path; nothing selective leaves a record by sync. `hale dna
  connect <record-url> --name <name> --as <position> --purpose <purpose>
  --classes <internal,customer,confidential> [--by <who>]` reads the
  other record's genesis (the root commit of its journal, fetched into a
  bare cache under `.hale/dna/peers/<name>.git`), refuses this record's
  own, and appends `connection.proposed` (`name`, `url`, `peer`,
  `position`, `purpose`, `classes`, `by`, `review_id`) with a Board
  Review `c-<name>-<n>`. The connection is in force once a `board`
  verdict approves that Review from someone other than its proposer —
  the host then settles the Review in the approver's name — and until
  `hale dna disconnect <name> --why <why>` appends `connection.closed`.
  `hale dna handoff <name> task <id> | receipt <digest> [--note …] [--as
  <who>]` writes one `handoff.received` row into the other record,
  pushed there under compare-and-swap: `handoff` (`h` and twelve hex
  digits of the origin genesis, kind and subject, so a fact crosses
  once), `origin_record`, `origin_url`, `origin_author`, `origin_row`
  (the source row's digest), `lineage` (the subject's rows here, as
  `kind#seq`), `purpose`, `position`, `via`, `kind`, `subject`, `class`,
  `fact`, `note`. This record then appends `handoff.published` (with
  `peer_row`, the commit it landed as) and, for a Task,
  `task.transfer_requested`. Only a handed Task crosses. A receipt
  crosses as its digest and class; its body never leaves this record. A
  fact whose class the connection does not carry is refused at the edge
  as `handoff.refused`, with nothing written across. The receiving record
  admits what arrives under its own policy: `hale dna handoff` lists a
  received handoff as admitted only under a connection in force back to
  its origin record that carries its class, and `hale dna handoff accept
  <id> [--as <who>]` appends `handoff.accepted` only for an admitted one.
  `hale dna handoff sync` reads every connection in force: each
  acceptance of a handoff published through it is admitted once, as
  `handoff.accepted_by_peer` and, for a Task, `task.transfer_accepted`.
  The Task settles only then, the rule retirement follows (GH #604 rule
  5). A closed connection is not read, so history stays in both records
  and nothing further is admitted.
- **An attributed external decision (GH #616).** A decision made by
  someone who does not run Hale — a manager, a client, an accountant —
  enters as a fact reported by a position inside the horizon, never as
  that party's verdict. `hale dna task decide <id> --decided-by <party>
  --via <channel> --evidence <digest> [--note …] [--as <reporter>]`
  appends one `decision.reported` row on the Task in the reporter's
  name: `reporter`, `decider`, `channel`, `evidence`, `scope` (the
  Task), `obligation`, `practice`, `policy` (the practice's digest),
  `accepted`, `why`, `note`. It is refused with nothing appended when
  the Task is not handed, when the reporter is not its assignee (GH #604
  rule 4), when the decider is the reporter, when a field is missing, or
  when the evidence is not a receipt the record holds. `hale dna receipt
  file <path> [--class internal|customer|confidential]` files one:
  internal text under `refs/dna/receipts/` with a `receipt.filed` row
  (`by`, `name`, `bytes`, `class`, `store`), a protected class through
  the knowledge service alone. Whether a report settles the Task is the
  obligation's **acceptance policy**: a practice named
  `acceptance/<obligation>`, ratified by the Board and not retired,
  whose text says `reported decisions: allowed`. The obligation is the
  class of obligation a person's job discharges; the leader's plan names
  it (`obligation:`) and `task.handed` carries it. The policy is read at
  admission and its answer kept in the row, so a later change of
  practice never rewrites what was admitted. Accepted, the Task is
  `decided`, and the projection shows the report as such: who reported,
  who decided, through what, on what evidence. Otherwise the Task stays
  handed, `hale dna board` lists it under *tasks waiting* with the
  reason, and its assignee closes it with `hale dna task done`. Without
  an obligation class or a practice in force, a report never suffices.
- **Schedules (GH #610).** An ask fired on an interval or a cron, in
  the organism's own name, taking the ordinary road. The org chart
  declares them in its `birth()` — `self.core.schedule(Schedule {
  id, every_ms | cron, ask, requires })` — and the substrate's clock
  (the org program's loop, `tick` with a millisecond monotonic clock)
  checks them. A declaration is a row, `schedule.declared <id>
  {action, every_ms, cron, ask, requires}`, when new or changed; a
  malformed cron (five fields, minute hour day-of-month month
  day-of-week; `*`, `a`, `a-b`, `*/n`, lists; ranges checked) is
  refused at declaration, never when it would first fire, with a
  `schedule.refused` row. An interval counts from the first tick and
  fires once per interval; a cron fires once in the UTC minute it
  names (day-of-month and day-of-week both restricted: either). Firing
  is `ask` with `Intent { id: "s:<id>/<n>", from: "schedule:<id>" }`,
  routed as `requires` says, and a `schedule.fired <id> {task, at}`
  row; a refusal by the membrane is `schedule.refused`. **Overlap:** a
  schedule never fires while the last Task it fired is open (born,
  pending, handed — anything but done or failed); the skip is a
  `schedule.skipped <id> {task, state, at}` row, never silent. `hale
  dna schedule pause <id>` / `resume <id>` append `schedule.paused` /
  `schedule.resumed` in your name from any clone; the organism reads
  them at its tick (a git journal is re-read from its ref at most
  every 5s) and a paused schedule never fires. `hale dna schedule`
  lists them as the record has them. At birth a schedule's state —
  its last Task, paused or not — is read back from the record. The
  optimize pass is a schedule: `optimize_every_ms` declares
  `optimize` (`action: "optimize"`) at birth and each run is a
  `schedule.fired optimize` row.
- **The optimize pass (GH #596 O).** On a cadence the org chart sets
  (`optimize_every_ms` on the substrate; 0 is never; the org program's
  loop ticks it with a millisecond monotonic clock), the substrate
  first asks the budget — the pass is model-backed work, and none is
  routed on an exhausted window: `optimize.refused <org>`, and the
  pass waits for the next window — then reads the record's structural signals
  — asks planned and how many took the defaults, concerns raised,
  grant contractions, verdicts refused, mutations rolled back — and
  asks the leader (`OptimizeRequested`, keyed by `org_id`) to walk the
  machinery, not the work. The leader answers with one small proposal
  or none (`OrgReviewed`), and the substrate journals `org.reviewed
  <org>` either way, with the signals it read: "if the state is clean,
  say so". A proposal enters as an ask in the leader's name and takes
  the whole road — planned, proposed, reviewed by the Board as an
  organization change. `hale dna` runs the pass on demand through the
  substrate's `optimize()`.
- **Grants layer by containment (GH #596).** The substrate may hold a
  `ceiling`: the grant above the child's — the organism's, the Board's
  to widen. A child's boundary reads its grant through the ceiling **as
  it is now** at every assessment: the classes both allow, the smaller
  magnitude, the stricter review (`intersect_grants`). Grants contract
  on their own (two failures in a row halve the ceiling), so a child
  born within a ceiling of 100 is bound by 50 the moment its parent
  contracts, and the record says so (`grant.contracted <parent>`,
  naming the children it now binds). A child born wider than its
  ceiling is the early error: `grant.refused <child>` names what is
  wider, and the ceiling binds it from birth. Law layers by adoption:
  every position lives under the org program's main, which adopts the
  organism's law, so a department's law can only add to it.
- **Grants delegate resources (GH #605).** A `Grant` also carries
  `amounts` (per-operation ceilings, `"USD 500.00 EUR 200"`; a currency
  not named may not be spent), `window_amounts` and `window` (`day`,
  `week` or `none`), `counterparties` and `routes` (space-separated;
  none named, none allowed), `expires` (unix seconds; 0 never) and an
  `authority_epoch`. Every field defaults to none, so a grant written
  before them spends nothing. They contain like the rest: the
  currencies both grant at the smaller ceilings, the smaller window
  limit, the parties and routes both name, the earlier expiry (never is
  later than any, so a child that never expires under a ceiling that does
  is wider in time), and the
  sum of the two epochs (`intersect_grants`); `resource_gap` names what
  is wider, and the birth refusal carries it. `permits(grant, spend,
  now)` checks a `Spend { id, amount, currency, counterparty, route }`
  predicate by predicate and names the field that fails (`expires`,
  `operation`, `currency`, `amount`, `counterparty`, `route`).
  `Dna.reserve(spend)` admits a spend against the effective grant and
  appends `grant.reserved <child> {op, amount, currency, counterparty,
  route, ceiling, epoch, at}`; the child's own window counts its
  reservations and the ceiling's window counts every child's under it.
  The remainder is read at a revision and the row is appended with
  `Journal.append_exact` at that revision — a writer that moved the
  record in between makes it stale, never re-appended at the tail — so
  two children cannot each spend the same remainder. A refusal is
  `grant.refused <child>` naming the field. `Dna.settle_spend(op,
  spent)` appends `grant.released {op, spent}` once, and the window
  counts what was spent from then on. A contraction advances the
  epoch, and so does `Dna.revoke_grant(by)` (the parent's only; nothing
  is left granted, `grant.revoked`); `Dna.admits(admission)` refuses an
  admission made under an older epoch (`grant.fenced`). The generated
  law adds `money_only_through_the_substrate: forbid reaches(positions,
  effects(money)) avoiding substrate`, with `effect money;` declared in
  the core; an organization's existing `dna/org/law.hl` is
  project-owned and gains the clause by hand. The money budget is
  distinct from the model budget.
- **The principal source (GH #612).** Who acts where a verdict, an
  intent or a task completion enters comes from one of two sources,
  `git config dna.principal`: `local` (the default) — the operator of
  the clone, as `--as` and the git identity say — or `oidc` — an
  identity provider. Under `oidc`, `hale dna ui` is a hosted head: it
  serves nothing without a session (`/api/*` answers 401, `/`
  redirects to `/auth/login`). A session comes from OpenID Connect's
  authorization-code flow against `dna.oidc.issuer` (https, or plain
  http on this machine only): the head discovers the issuer's
  endpoints, sends the browser to its authorize endpoint with a random
  `state` and `nonce` (`std::os::getrandom`), and exchanges the
  returned code at the token endpoint itself with the client's secret
  (`dna.oidc.client`, `HALE_DNA_OIDC_SECRET` read into a sealed locus,
  `dna.oidc.redirect` as the callback). That ID token came over TLS from
  the issuer's own endpoint, which authenticates it (OpenID Connect Core
  §3.1.3.7, rule 6) in place of an RS256 signature the standard library
  cannot verify; `claims_refusal` checks the issuer, the client among the
  audience, the expiry and the nonce. The subject — never an email alone
  — maps to a member through a reviewed mapping, `git config --add
  dna.oidc.member "<subject>=<name>"`; an unmapped subject gets no
  session. A sign-in's state is used once and expires in ten minutes; a
  session is a random 256-bit id in an `HttpOnly; SameSite=Lax` cookie
  (`Secure` when the callback is https) and lasts eight hours or until
  `/auth/logout`. With a session, a verdict acts as the member (`--as`,
  and `--authority board` when `dna.oidc.board` names them, `reviewer`
  otherwise) and an intent is asked by them (`hale dna ask --as`), the
  row's author too; the form's own `as` field is ignored. The head never
  acts in its own name. It speaks plain HTTP: TLS is a reverse proxy in
  front of it. Rows arriving by sync remain admitted under `dna.trust`
  (GH #604 rule 6); a synced row claiming a rank is not a sign-in.
- **The membrane over the record.** From a clone with no organism,
  `hale dna ask` appends `intent.requested` (the body: outcome, from,
  to) and a verdict appends `review.verdict` (the body: the verdict as
  the socket membrane carries it), each in the appender's git identity;
  the host beside the organism relays unanswered rows onto the
  membrane once, admitting each under `dna.trust` (GH #604 rule 6): `local`,
  the default, trusts every writer to the record — the operator's
  profile; `signed` admits a row only when its commit carries a
  signature git verifies (`git verify-commit`; git's keyring or
  allowed-signers file is the list of who may act), and a row that
  does not is refused in the record in the host's name
  (`intent.refused`, `review.refused`, `concern.refused`: "unverified
  writer") and never relayed. Under `signed`, rows this clone writes
  are signed commits. Signing is one mechanism for this edge, not the
  boundary itself: a hosted head is admitted through a reviewed mapping
  of principals to authority and acts as the user, never as itself. and the organism's answers (`intent.offered`,
  `task.born`, `review.settled`, `review.refused`) return the same way.
  A row is answered when a later row of the answering kind names its
  entity. `hale dna ask --no-wait` appends and returns.

`.hale/dna/` holds only what is not the record: the membrane sockets,
the status projection, worktrees, scratch inputs to the toolchain.
Deleting it loses nothing the record holds.

**The socket routes are relative on both sides.** A Unix address holds
108 bytes, path included, and `.hale/dna/hale-dna.review.verdict.sock`
already spends 39 of them, so an absolute route puts an ordinary
project path over the limit. The organization binds these names
relative to the root it runs in; the membrane client is run with the
project root as its working directory and given the same relative
routes, and a node's instances connect to `.hale/node/concern.raised.sock`
the same way. A project's path therefore has no length rule.

## Event kinds

`application.attached`, `structure.observed`, `responsibility.proposed`,
`law.deferred`, `intent.requested`, `intent.offered`, `intent.refused`,
`review.verdict`, `task.born`,
`task.<state>`, `mutation.proposed`, `mutation.worktree`,
`mutation.located`, `mutation.candidate`, `mutation.<disposition>`,
`mutation.applied`, `mutation.retained`, `mutation.rolled_back`,
`mutation.rejected`, `mutation.revise`, `mutation.refused`,
`mutation.apply_retried`, `knowledge.retired`, `task.planned`,
`grant.refused`, `grant.contracted`, `task.handed`, `org.reviewed`,
`task.resumed`, `intent.unrecovered`, `task.reassigned`, `person.retired`,
`concern.refused`, `body.claimed`, `body.released`, `body.provisioned`,
`secret.rotated`, `body.credential_missing`, `body.credential_present`,
`schedule.declared`, `schedule.refused`, `schedule.fired`,
`schedule.skipped`, `schedule.paused`, `schedule.resumed`,
`grant.reserved`, `grant.released`, `grant.fenced`, `grant.revoked`,
`receipt.classified`, `receipt.withheld`, `receipt.disclosed`, `receipt.read`,
`receipt.read_refused`, `receipt.held`, `receipt.hold_released`, `receipt.redacted`,
`optimize.refused`,
`mutation.failed`, `effect.requested`, `effect.result`,
`evidence.<step>`, `evidence.magnitude`, `review.requested`,
`review.settled`, `review.refused`, `expression.restart_requested`,
`expression.deployed`, `expression.restarted`, `expression.observed`,
`expression.crashed`, `pressure.raised`, `pressure.remeasured`,
`appendage.proposed`, `model.called`, `budget.exhausted`, `knowledge.proposed`,
`knowledge.ratified`, `knowledge.declined`, `knowledge.refused`, `knowledge.consulted`,
`concern.requested`, `concern.raised`, `concern.proposed`, `github.pr`, `github.commented`,
`mutation.topology`, `fleet.deploy`, `instance.up`, `instance.exited`,
`review.reasoned` (the deciding verdict's comment — a person's note or
the Leader's reasoning — right after `review.settled`; `hale dna
review <id>` renders it as `why:`). Their bodies are documented in the guide's reference
chapter; the set grows by ordinary change, and a reader that meets an
unknown kind must keep walking.

## Storage interfaces

`Journal` (ordered append with an expected revision, read by index,
chain verification), `Coordination` (leases with fencing tokens) and
`Receipts` (content-addressed store and read) are interfaces in the
core. The git-backed implementations are the ones an assembly wires
for an organism; the in-memory ones exist for tests. Protected bodies
go through `ReceiptVault` to the knowledge service's `ProtectedBodies`
(`MemProtected`, `PqProtected`) instead (GH #606).

## The organization

The organism is an organization written in Hale (GH #566 F2): a
program at `dna/org` that `hale dna init` generates and `hale dna run`
runs. The application it oversees is not modified by `init` and
contains none of it; it is observed like any Hale binary. The
manifest declares two environments, the application's and the
organization's (`[claims] no_base = true`; each adopts its own law).

- **Positions are loci; routing is the bus.** `Board` (the human
  authority: intent enters through it, escalations and reports leave
  through it, it owns every grant), `Leader` (a model-backed position
  holding the project's grant: it decides the Reviews inside the grant
  by reading the source diff and the semantic diff, and its verdict is
  a model call with evidence), and the substrate `Dna` (the record,
  the gateways, verification, the editing position, the Reviews). A
  Review is announced as a typed `ReviewRequested` fact carrying what
  a deciding position needs.
- **Authorities are ranked**: `board` (4, `maintainer` is its older
  name), `leader` (3), `supervisor` (2), `reviewer` (1). A claimed
  authority satisfies a requirement of its rank or below; an unknown
  name satisfies only itself.
- **Who decides.** `OrgPolicy`: a change that touches law, widens
  effects or crosses ownership, or is of class `organization`,
  `constitutional`, `process-policy` or `topology`, requires the
  Board; a change outside the grant (disposition `escalate`) requires
  the Board; everything else inside the grant requires the Leader.
- **The foundational law** (`dna/org/law.hl`, generated, extendable,
  never weakened): nothing applies except through the substrate
  (`forbid reaches(positions, effects(genome_apply)) avoiding
  substrate`); the editing position reaches neither `repo_write`,
  `worktree_io`, `genome_apply` nor the Knowledge; the Leader's
  verdict reaches the genome only through the substrate; credentials
  are sealed. The claim engine follows bus edges, so a position that
  could reach an effect through a published fact is a build failure
  with the path as its witness.
- **Hosts.** `hale dna run` builds and runs the organization with iris
  attached; `hale dna dev` runs the application under the same host
  too, rebuilds and restarts it on an apply, records
  `expression.restarted` in the host's name, watches the window and
  reports on the membrane. The host writes `org.pid` and `app.pid`
  under `.hale/dna`. Under `run` alone a restart request is logged,
  not answered: expressing an application deployed elsewhere is a
  deployment gateway's job.
- **One body per record: the body lease.** A record admits one body
  at a time (bounded attachment; the initial controller model). The
  host takes `refs/dna/lease/body` before it builds or runs anything
  — at the record's remote when it has one, so two hosts on two
  clones fence each other through the remote itself (the lease blob
  is pushed with `--force-with-lease`, git's compare-and-swap), in
  the clone otherwise. The blob is a `GitLeases` lease
  (`holder\ntoken\nexpires\npresent`), the holder
  `user@host:<clone>`, so the same clone restarting is the same
  holder and takes its lease back at once; a second clone is refused
  with who holds it and exits 3. The lease lives 30s and is renewed
  when a third of that is gone; the host **asserts it at the top of
  every tick, before it relays, restarts or applies**, and stops
  itself (exit 3, the organization with it) when the lease is
  someone else's or released. While the remote cannot be reached the
  lease is kept unrenewed until it expires, then the host stops: a
  partitioned body executes nothing past its TTL. Taking the lease
  is a row (`body.claimed <holder> {token, forced, by}`), giving it
  up at exit another (`body.released`). `hale dna body` reads the
  lease; `hale dna body claim --force` releases a live lease that
  belongs to a body that is gone, as a `body.claimed` row with
  `forced: true` in the forcer's name (that body stops when it next
  asserts; the next `hale dna run` takes the lease); `hale dna body
  release [--force]` releases it explicitly.
- **The combination is detected, never stored.** An organism is one
  point in a space of independent capabilities — where the body
  runs, whether a hosted head exists, whether a fleet expresses it,
  which records it is connected to — declared by its pieces. `hale
  dna profile` prints one line per axis from the pieces alone: the
  record's remote, the body lease and its last tick, `[dna] fleet`,
  the knowledge store's environment, `dna.trust`, `dna.github`; the
  status projection carries a `profile:` and a `body:` line. `hale
  dna new --profile local|remote-body [--remote <url>] [--body
  <user@host>]` sets pieces (a remote, `dna.body`); the profile name
  is not written anywhere. Profiles are examples of combinations,
  not a closed set; the toolchain refuses only what breaks a
  commitment (a second body on one record).
- **The body on a server.** `hale dna body provision <user@host>
  [--dsn <url>] [--dir <path>] [--dry-run]` makes a body over ssh, in
  order: the toolchain `hale.lock` pins (installed with the site's
  installer at that version, verified, or stop), the record's remote
  cloned (fetched into a clone that exists), `vendor/dna` and the
  record brought up, the knowledge database from `dna/compose.yaml`
  or a DSN (which goes into the body's env file, never the record),
  and a systemd user unit `hale-dna-<project>` supervising `hale dna
  dev . --no-iris` — on one server the body is the organization, the
  application and the knowledge service under one host — with
  `Restart=on-failure` so a failure flows up one more level. It stops
  before writing anything when the record has no remote or one local
  to this machine, when ssh cannot reach the host, or when the host
  lacks git, curl, systemd, or (without a DSN) docker compose. It
  records `dna.body` and `dna.body.dir` here and a `body.provisioned
  <user@host> {dir, toolchain, knowledge, by}` row. `--dry-run` prints
  the exact script. `hale dna body start|stop|logs [--body …]` reach
  the unit over ssh. Postgres, Docker, ssh and systemd are the
  reference setup; the definition admits other implementations.
- **Secrets.** `hale dna secret set <NAME> [--body <user@host>]`
  reads the value from stdin — never argv (a `NAME=value` argument is
  refused), never the record — and writes `NAME=value` into
  `~/.config/hale-dna/<project>.env` (mode 600; one line per name,
  the newest) on the body over ssh's stdin, or on this machine. The
  host loads that file into its children's environment. `secret
  rotate <NAME>` is the same for a name already set. The record gets
  `secret.rotated <NAME> {where, by}` and nothing else. At start the
  host checks the credentials its catalog names (`env_var` in
  `dna/org/models.hl`): when none is set in its environment or the
  file it says so and appends `body.credential_missing model
  {any_of}`; `status` and `board` carry "no credential for the
  model" until a start finds one and appends
  `body.credential_present`.

## The organization evolves

A change of class `organization` edits the organization's own seed
(`Mutation.seed`, `dna/org`), through the same pipeline as a change
to the application: a worktree, the editing position confined to that
seed, verification of that seed (its own base artifact cut at the same
moment), the semantic diff of the organization (positions are loci,
routes are subscriptions and publications, capabilities are effect
classes), a Review that is the Board's. Applying it restarts the
organization (the restart request names the seed; the host answers
one for `dna/org` by rebuilding and restarting the organization
itself, which records `expression.restarted` at birth); the window
then judges the new organization.

Growth is initiative, not reflex: when one source raises pressure
`appendage_threshold` times, the assembly journals
`appendage.proposed` and — with `initiative` on — proposes a growth
Mutation of the organization's seed (`appendage.candidate` names
it). Nothing is grown until the Board approves the candidate commit.

`hale dna board` is the Board's queue: the Reviews only it can settle,
the proposals, the last report. `hale dna report` appends
`report.filed`, in the name of whoever asked, summarizing the record
since the previous report (proposed, reviewed, applied, retained,
rolled back, rejected, escalated, pressure, proposals, model calls and
cost, settlements). `hale dna pressure` lists the pressure raised and
answered; `hale dna pressure raise <source> <what>` publishes one
signal on the membrane's fourth topic, `PressureRaised`
(`dna.pressure.raised`).

## The editing position

`SourceEditor.perform` plans the files an objective is about — every
listed file the objective names, else the quick tier's plan from the
listing, one file per line; only a listed file is ever a target —
edits each (one model call per file, the request naming the file as
`target`), formats and checks the seed, and, when the check fails,
tries again with the diagnostics in the prompt, up to `max_tries`
in all (default 3). Each try is an attempt id (`<work>/a<n>`), so
every try's model calls are evidence in the record. The result names
the files changed and the tries taken; a proposal that does not check
within the bound is a failed Attempt with the last diagnostics.

## Models

The organization's models are a catalog in source (GH #583 M1):
`dna/org/models.hl`, generated by `init` and owned by the project.

- **The seam.** `ModelBackend` (`identity`, `model_name`, `allows`,
  `capacity_bytes`, `complete`) is what every adapter implements;
  every call publishes `ModelCalled`, which the substrate journals as
  `model.called` with the time of the call (`at`). **The prompt is a
  receipt (GH #611):** the evidence carries the prompt and the context
  as sent, and the substrate files each as a receipt in the record
  (`refs/dna/receipts`) under the `prompt_digest` / `context_digest`
  the row names — only a body that
  hashes to its digest, and receipts are content-addressed, so an
  identical brief is stored once — recording `bodies: stored` (or
  `none` for a call with neither). An organism whose receipts are not
  the record's (a file store: a fixture, an embedded use with no
  record) files nothing and records `bodies: none (receipts are not the
  record's: file)`. A `customer`-class call files no
  body: `bodies: withheld (data class customer)`, and its digests are
  all the record keeps. `hale dna history <attempt>` renders, under each
  `model.called` row of that attempt, `prompt <digest> (<n> bytes)`, exactly
  those bytes from the receipt, and `end of prompt` (the same for the
  context) — never a re-composition; a withheld body says so, and a
  body this clone lacks says `hale dna sync` fetches it. A wider
  history (`hale dna history m1`) names the calls and leaves their
  bodies to each attempt's own, since an editor's prompt carries whole
  files. The row says `stored` only when the store gives the body back.
  Routers return
  data, never a backend. Adapters: `OpenAiChat` (the OpenAI chat
  shape: OpenAI, OpenRouter, vLLM, Ollama; `adapter: openai-chat`),
  `AnthropicMessages` (the native Messages API: `system` beside
  `messages`, `max_tokens` required and defaulted to 4096, the reply's
  text blocks joined, `anthropic-version` sent; `adapter:
  anthropic-messages`), `LocalModel` (the OpenAI shape to this
  machine; no credential, no `external_model`, any data class),
  `FakeModel` (scripted: `answer`, `answer_file`, `answers_dir`,
  `answer_role`; `fail_after` refuses after that many calls). A hosted
  adapter's `complete` carries `external_model`; its credential is a
  sealed `HostedCredential` that names its source (`env_var`) and its
  `scheme` — `bearer` (an `Authorization: Bearer` header) or
  `x-api-key` — and never returns the bytes: the credential composes
  the header, no adapter does. An API error is refused as `http
  <status> <the API's message>`; a transport failure as `http 0
  <kind> <detail>`.
- **The harness.** `HarnessModel` runs an installed coding harness
  (`command: claude`, or `codex` with `output: text` and `exec
  --skip-git-repo-check --sandbox workspace-write`, because a
  non-interactive codex is read-only by default and would answer
  without ever editing its export) per request with
  its own tools on, as a backend that `works_in_place`: for a
  source-editing request the editor makes an EXPORT of the Mutation's
  worktree (a plain directory with the same files, never `.git`, never
  `.hale`), runs the harness with the export as its cwd and the whole
  objective as its prompt, and imports the export's diff back under
  the grant — a file that differs or is new is written through the
  tools, one that is gone is removed, a change outside the grant is
  counted (`outside_grant`) and left behind; `files_changed` is
  derived from that diff, never from the harness's answer. A file the
  worktree does not have is new whatever it holds, empty included. A
  written file's parent directories are made under the grant first, so
  a harness may add a module in a directory the worktree does not have
  yet, and **an in-grant write or removal that fails is the attempt's
  failure** (`the import was incomplete: …`), never a candidate
  carrying part of the change. Then the
  same fmt, check, retry (the diagnostics in the next prompt) and
  assessment as the file-by-file flow. An answer-only request (a
  review) runs in an empty directory of its own. `complete` carries
  `external_model` and `harness_run`; evidence is one `model.called`
  row per call with the harness's own cost summary
  (`total_cost_usd`, `usage`), `tool_grant: harness @export`, and
  `confinement=<kind>` in `params`.
- **The boundary is stated exactly.** The export is not a sandbox: a
  process whose cwd is the export can still reach whatever the
  operator's account can. What DNA guarantees is that *the genome is
  unreachable from the harness process*. A `Confinement` (`kind`,
  `available`, `wrap(argv, cwd, mask)`) supplies it: `Bubblewrap`
  (Linux: the filesystem as the operator sees it — the harness's own
  home state, the toolchain, the network — with the repository and
  the worktree replaced by empty tmpfs mounts, `--die-with-parent`)
  or `NoConfinement`. **The genome is the model's own, not the
  request's**: `HarnessModel { genome, genome_env }` reads it at birth
  (the host exports `HALE_DNA_GENOME`), every call masks it, and a
  request's `mask` adds what that call knows besides — the worktree an
  editor exported. Every verb the host runs is given
  `HALE_DNA_GENOME`, not only the organization under `run` and `dev`:
  `hale dna models` probes the catalog in a process of its own, and a
  confined harness with nothing to mask is refused. An answer-only role (a review, a classification,
  the catalog's probe) carries no request mask, so a genome on the
  model itself is what makes the guarantee true for them. **A
  confinement with nothing to mask is refused**, not run, so the claim
  and the fact cannot diverge. A harness whose confinement is
  unavailable is refused (`unconfined harness not allowed`) unless the
  org chart's `allow_unconfined` says otherwise; the evidence records
  which it was, and `masked=<n>` how many paths the boundary covered. On every platform the assembly also checks that the
  repository's head and the worktree's head did not move during the
  attempt and fails the Mutation (`mutation.failed`: `the genome
  moved during the attempt`) if they did: not a boundary, but a
  bypass becomes a loud refusal. Outside the genome the harness has
  exactly the operator's account, which is what running it by hand
  has. The `Repository` interface names its `root` for the mask.
- **The tape.** `RecordedModel { dir, mode, inner }` wraps any
  backend (`dir_env` / `mode_env` name environment variables it reads
  into `dir` / `mode` at birth, so a catalog function computes no
  string: dna/FRICTION.md F.17). The key is `sha256` over the fields that identify a
  request — role, the inner's name and model, the prompt and context
  digests, the data class, the grant normalized (`… @grant`: its path
  is where it ran, not what it was) and, for a backend that works in
  place, a digest of the workspace's starting tree. An identical
  request made again in one run — a retried Work planned a second time —
  is keyed by its occurrence as well (`occurrence=n` for the n-th; the
  first keeps the plain key), so a recording that got two answers
  replays both in order, and a miss on a later occurrence says it was
  never recorded. `record` makes the
  tape's directory before it writes anything into it, and an entry is
  counted only once every file of it has landed: a patch or an entry
  that could not be written is a refusal (`cannot write the tape: …`),
  because an entry claiming a patch that is not there replays as a
  miss on a request the tape appears to hold. `record` forwards
  to `inner` (whose evidence is the call's) and writes `<key>.json`
  with every keyed field in clear and the answer, and for a workspace
  `<key>.patch`: what the backend changed there, before any import,
  so a replay reproduces the edit and everything after it runs for
  real. `replay` answers from the directory (evidence `adapter:
  recorded`, `params: tape=<key> mode=replay`), applies the patch to
  the workspace, and refuses a miss naming the request and, when an
  entry shares the prompt digest, the fields that differed (`tape
  miss <key> (role …, backend …, …): nearest <key> differs in:
  context_digest tree`). Replay is pure; record reaches whatever
  `inner` reaches. Anything that varies between runs must stay out of
  the key, and recording finds what does: the editor therefore strips
  the worktree's path from the diagnostics it feeds back. The tape is
  a checked-in fixture, not a store. It proves how the organization
  handles recorded outcomes; model quality is only ever tested by a
  fresh run.
- **The fixture.** `dna/acceptance/trio`: a gateway (the root seed),
  an api and a worker under a plan of four instances on two nodes
  with routes between them and claims across them
  (`require_publishes` on the gateway and the api, `forbid_reaches`
  gateway → worker avoiding the api). Its organization, with every
  backend a `RecordedModel` over the real adapter
  (`dna/acceptance/trio.fixture/catalog.hl`; mode from
  `HALE_DNA_TAPE`, the tape from `HALE_DNA_TAPE_DIR`), is driven by
  scripted asks end to end: a change to the gateway approved and
  deployed to both nodes, the organization grown by a supervisor
  under pressure from the worker and restarted with it, and a change
  to one service that still checks but breaks the fleet's claim
  denied — every model call answered from the tape, keyless. It is
  the acceptance for every later change to the organization;
  `HALE_DNA_TAPE=record` with a key re-records it.
- **The catalog is source.** A backend is a constructor function
  (`frontier()`, `fast()`, `desk()` …); a position's router is a
  function composed from them (`leader_models()`, `editor_models()`,
  `agent_models()`); the organization's main takes each position's
  router from the catalog and names no adapter inline. A new position
  is one more function; a provider switch is one file; `hale check`
  validates it and the law keeps the concrete types at every
  boundary.
- **Discovery.** `init` and `new` look at the environment
  (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`) and `PATH` (`ollama`, whose
  first listed model becomes the desk model; `claude`, else `codex`,
  becomes `harness()`), write the catalog for what is there
  (`AnthropicMessages` with `claude-opus-5` / `claude-haiku-4-5` when
  its key is present, else `OpenAiChat` with `gpt-4o` /
  `gpt-4o-mini`, a placeholder key when none is set; with a harness
  the editor's and the agent's `quick` tier is the harness, and with
  no key every model-backed slot is), and print what each position
  was given. Nothing found is fine: a hosted backend without its key
  is not permitted, and every Review waits for the Board. `upgrade`
  writes a catalog for an organization that predates it and says what
  to point at it; it never edits the organization's main.
- **The budget has one owner.** The catalog declares `org_budget()`,
  a `BudgetPolicy` (`window`: `day`, `week` or `none`;
  `allowance_micros`, 0 = unmetered) as data. The substrate owns the
  one `Budget`: every `model.called` row is accounted at its time, the
  counters are rehydrated from the record at birth, and when a window
  is exhausted intent is refused (`intent.refused`: `budget
  exhausted (spent … of … micro-dollars this day in … call(s))`),
  `budget.exhausted` is journaled once per window (`detail`,
  `spent_micros`, `allowance_micros`, `window`, `at`), the membrane is
  told, and a Review is not announced to the model-backed positions —
  it waits for the Board, which needs no model. A `ModelRouter` counts
  what its calls cost and has no allowance of its own.
- **`hale dna models`** builds the catalog beside a one-line main in
  `.hale/dna/probe`, runs its `probe_catalog()` from the project root
  with the organization's environment, and prints one line per
  backend: the catalog name, the router-facing slot, the model, and
  the answer — `ok`, elapsed, cost and the first line of the reply;
  `refused: …`; or `not permitted (no credential present)`. The
  organization is not started, its membrane not touched, and nothing
  is journaled: a probe is nobody's Attempt.

## Knowledge

The knowledge graph is a service (GH #583 K1). Two halves, one
authority.

- **The record holds the decided half.** A proposal is a document —
  canonical JSON with a fixed field order (`kind`, `text`, `author`,
  `target`, `provenance`) — filed as a receipt under its sha256, so a
  digest in the record always resolves to recoverable content.
  `Dna.propose_knowledge(idea, target)` files the receipt, appends
  `knowledge.proposed <digest>` (body: `digest`, `review_id`, `kind`,
  `author`, `target`, `class`, `at`), and births a Review pinned to
  that digest (`review_id` = `k:` + the digest's first twelve hex
  digits; `required_authority: board` — ratified knowledge is the
  organization's; the `review.requested` body carries
  `knowledge_digest`). The tower rule classifies the binding from who
  proposes and where it binds — a parent's idea bound to a child is a
  `goal`, a child's bound to a parent a `concern`, one's own an
  `initiative` — and a lateral proposal (siblings) is refused with
  `knowledge.refused` and no Review. A verdict naming another digest
  is refused by the Review; a Leader's does not satisfy the Board's
  requirement. **The authoritative event is `knowledge.ratified
  <digest>`**, appended by the assembly on `review.settled` with
  `approve` (body: `digest`, `review_id`, `outcome`, `decided_by`,
  `kind`, `target`, `class`, `at`); any other outcome appends
  `knowledge.declined`. A pending knowledge Review is re-born from the
  record at birth like a mutation's.
- **The store holds the live half.** `dna/knowledge` (embedded in the
  toolchain beside the core, with pond's Postgres driver pinned under
  `dna/pond`) declares `KnowledgeStore`: `open`, `watermark` /
  `set_watermark` (the next record seq to apply; it only advances),
  `upsert(idea, ratified_seq)` (idempotent by id, which is the
  digest), `bind`, `link`, `idea(id)`, `context_ids(target, budget)`
  (accepted ideas bound to the target or any prefix of its path —
  goals flow down, initiatives stay local — in ratification order),
  `count(what)`. `Pq` is Postgres (six tables: `knowledge_meta`,
  `knowledge_ideas`, `knowledge_bindings`, `knowledge_edges`,
  `knowledge_structure`, `knowledge_signals`; the schema migrated at
  `open`; every write an upsert; rows come back as
  JSON built by the server, since the driver's tab- and
  newline-separated rows cannot carry an ordinary paragraph; a signal
  counts once per record row, keyed on that row's sequence, because
  the projection write and the watermark advance are separate and a
  crash between them replays the row); `Mem` is the
  same contract in memory. **The service is a consumer of the
  record**: `apply_record(store, journal, receipts)` walks
  `knowledge.*` rows from the watermark, resolves each digest to its
  receipt, upserts (`ratified` accepted with its binding; `proposed`
  and `declined` kept as such, never over a ratification), and
  advances the watermark row by row, so after a crash it resumes and
  converges. Nothing in the store becomes ratified except from a
  `knowledge.ratified` row: git is the sole authority; the store can
  lag, never disagree. The observed and inferred tier (K3) lives only
  in Postgres and is backed up like any Postgres; the ratified tier
  rebuilds from git.
- **The record is the scope of its store.** A store is opened for one
  record, named by the record's identity — the sha of the journal's
  first commit, the same in every clone of the record and different
  for every record (`GitJournal.genesis()`; `scope(record)` on the
  store before `open`, `record()` to read it back). `Pq` keeps one
  Postgres schema per record, `dna_<identity>`, selected for the
  session at `open` (with `public` behind it for the vector type), so
  two records on one database — an operator's `HALE_DNA_KNOWLEDGE_DSN`
  shared across projects — each see only their own ideas, bindings,
  structure, signals and watermark; `hale dna dev`'s compose database
  is one record's anyway. The schema's `knowledge_meta` carries a
  `record` row naming its owner, written on first open and checked on
  every open: a schema that names another record is refused (`scope:
  schema … belongs to record …; it is not read`), never read. A store
  in `public` from before stores were scoped is refused with the way
  forward (drop its tables or the database; the projection rebuilds
  from the record), never read as any record's. A record with no
  rows yet has no identity: its service opens under `dna_unscoped`,
  where nothing is ever applied, and moves to the record's own schema
  on the request after its first row lands. The summary reports the
  scope (`"scope"`).
- **The service program.** `dna/knowledge/service` (`hale dna
  knowledge [project] [--port N]`, default 8791): applies the record
  on every request (the reader sees it as it is now) and answers over
  HTTP. **A question it cannot answer is a refusal, never an empty
  answer**: a package built from failing reads, or served while the
  record's projection is stuck, is indistinguishable from "there is no
  knowledge here" — so a read error or an `apply_record` error is 503
  with the reason, and the summary carries whatever the counts hit.
  The store is opened on the first request rather than at birth
  — a database that is down must not hold the surface closed, because
  the surface is where an operator reads that it is down — and the
  connection is asked on each request afterwards (`healthy`), because
  an open that succeeded once is not a connection that still answers:
  a session dropped by a restart or a failover is re-established
  (`reopen`), and only a store that cannot be reached at all answers
  503. A package is never built from a store whose queries are
  failing — `GET /` (store kind,
  watermark, record revision, counts), `GET
  /context?target=<locus path>&budget=<n>` (the bounded package: the
  ids included, their ideas with text, author and `ratified_seq`, and
  the store's revision, and a digest over the target and the ids — what is handed over, never the revision, which varies between runs and would make a tape unable to answer the same request twice), `GET /idea/<digest>`,
  `POST /apply`. `HALE_DNA_KNOWLEDGE_DSN` names the store: a
  `postgres://user:password@host:port/database?sslmode=…` URL, or
  `memory` for a store that lives only as long as the process; unset
  is a refusal that says so.
- **The design, as proposals (GH #596 C).** `init` seeds the
  toolchain's practices about how a DNA organization works — the
  design principles, the evolution pattern, structure follows intent,
  standard equipment, the structural signals, signaling, the optimize
  pass, software delivery — as `knowledge.proposed` rows with receipts
  bound to `org`, **one Review per practice** (`k:<12hex>`, grouped
  `design` in `hale dna review`'s listing; `hale dna review design
  approve|reject` is a convenience that decides each pending one in
  turn, never a batch object in the record). A Review pins one digest
  and settles with one outcome, so the Board decides practice by
  practice, and nothing is ratified by the toolchain. Each receipt
  carries a stable `name` (`design/<slug>`) and the toolchain version.
  **Supersession is the Board's decision, not a side effect.**
  Declining a proposal retires nothing (the projection keeps a
  ratified idea a later `knowledge.declined` names). A proposal whose
  receipt carries `supersedes: <digest>` retires that digest when it
  is ratified — the assembly appends `knowledge.retired <old>` — and
  leaves it in force when it is declined; a `retirement` proposal
  carries only that. A replacement is ratified only while what it
  replaces is still active: an approval of a proposal whose
  `supersedes` names a digest already retired is refused
  (`knowledge.refused <digest>`, with what retired it) rather than
  ratified — otherwise two replacements of one version would both be
  served, the first never retired. `hale dna upgrade` proposes each
  practice whose current text is not the latest proposed under its
  name, superseding the active version under that name (a pending or
  declined proposal is not a predecessor), one replacement at a time:
  a practice whose latest proposal is still before the Board waits
  (`upgrade` says so), and a refused one is proposed again against
  what is active now. The projection
  retires an idea by marking it not accepted: it leaves every package
  and stays readable by digest.
- **The charter (GH #596 L).** `init` writes `dna/org/charter.hl`, a
  function returning text like `purpose`: the leader's brief, saying
  that it is the organism's architect — it proposes, the Board
  decides — and what it must know before it plans an ask or decides a
  Review. Project-owned; a change is a reviewed change to the
  organism. `upgrade` writes it for an organization from before it.
- **The leader reads its brief at both moments it thinks.** The org
  chart hands the `Leader` its `charter` and `purpose` (the program's
  own text) and a `KnowledgeClient`; before a review and before a
  plan it composes its brief — `CHARTER`, `PURPOSE`, the `LAW` as the
  genome holds it at HEAD, and `PRACTICES` from the package for `org`
  when the service has any — and puts it above the diffs. The model
  call carries `knowledge_bindings` naming the package, so the
  evidence of every decision that read it says so.
- **`HALE_DNA_DISCOVER=off`** makes `init`'s discovery find nothing —
  no key, no local model, no harness — so a fixture on a developer's
  machine makes the organization CI makes: one whose leader has no
  model that answers, whose plans take the defaults, and which spends
  nothing. The CLI fixtures set it.
- **An ask is planned before it becomes a Mutation.** With `planned:
  true` on the substrate (the generated org chart says so), routed
  edit Work is not a Mutation of class `application` at once: the
  substrate publishes `PlanRequested`, the leader answers
  `TaskPlanned`, and the substrate journals `task.planned <task>`
  (kind — `organism`, `appendage`, `product` or `person` —
  `change_class`, `target`, `count`, the model's narrative, the
  package read, `parsed`, and `class_applied`) and proceeds under the
  class and target the plan names. **The kind decides whose change
  it is.** A plan of kind `organism` is a change to the organism
  itself, and that is class `organization` whatever the plan called
  it: it is expressed from the organism's seed and assessed under the
  organism's policy, so it is the Board's to decide. Class
  `organization` for a child that is not the organism is a
  contradiction and is refused before any editor is placed
  (`task.refused`, the Work fails, no Mutation). **A plan only splits
  and classifies; it never widens what was asked.** An answer that names no kind and no class leaves
  the defaults standing — an appendage, class `application`, the
  ask's own target — and the record says it did not parse, so an
  organization with no model that answers behaves as before. A plan
  of kind `person` settles the Work as handed rather than mutating
  anything. A `task.planned` row is not a settlement: the task's state
  is its last other `task.*` row.
- **Work survives a restart (GH #604).** The record holds a Task's
  birth before anything runs: the id is minted, `task.born` is
  appended, and only then is the Task born — a birth the record
  refuses stops there (`intent.refused`), and nothing has run. On
  restart, a Task born and not settled re-enters the tower from its
  last durable state (`task.resumed <task>`): under the plan in the
  record when there is one, never replanned; planned for the first
  time when there is none. A Task whose Mutation was in flight settles
  `failed` with the Mutation; one whose Mutation is beyond proposal
  waits on that Mutation's outcome, and settles from it when the Work
  that would have settled it is gone. A handed Task is a person's and
  waits. An `intent.offered` with no `task.born` naming it — the shape
  from before this rule — is noted (`intent.unrecovered`) and never
  re-offered: work may already have run.
- **Dev relies on docker compose.** `init` writes `dna/compose.yaml`
  (the `knowledge-db` service, `pgvector/pgvector:pg16`, a named
  volume `hale-dna-<project>-knowledge`, a host port in 54xx from the
  project's name); it is part of the genome. `hale dna dev` runs
  `docker compose -f dna/compose.yaml up -d --wait knowledge-db`,
  derives the DSN from the published port, starts the knowledge
  service beside the organization (`knowledge.pid`, `knowledge.dsn`,
  `knowledge.log` under `.hale/dna`; `HALE_DNA_KNOWLEDGE_PORT`), and
  stops it with the rest. An operator's `HALE_DNA_KNOWLEDGE_DSN`
  wins. With compose not on PATH, or no compose file, the host says
  exactly which it needs and runs without a knowledge service. `hale
  dna run` starts no service: beyond one machine the service is an
  instance in the plan against a Postgres of the operator's.
- **Knowledge changes later work (K2).** The substrate holds a
  `KnowledgeClient` (`url`, or `url_env` read at birth — the host
  sets `HALE_DNA_KNOWLEDGE_URL` under `dev`; `budget`), the owner's
  line to the service; the editor never holds one, which the law
  states (`group knowledge = { dna::Knowledge, dna::KnowledgeClient
  }`, `editors_never_learn`). When a Mutation opens, the substrate
  asks the service for the package of the change's target in the
  tower — `org` for an organization change, `org/<child>` for the
  application, `org/<child>/<seed>` for a seed inside it — and
  journals `knowledge.consulted <mutation>` (`target`, `digest`,
  `revision`, `included_n`, `included`, `error`) whenever a client is
  configured, answer or not. The package's ideas are folded into the
  objective the editor receives (`objective_with`: the ask, then a
  `PRACTICES (ratified knowledge for <target>, package <digest>):`
  block, one idea per line) — the record, the commit message and the
  Mutation keep the ask itself — and the request names the package
  (`context_digest`, `knowledge_bindings: package:<digest> <id>…`),
  which every model call of the attempt carries into its `model.called`
  row. No service, or a service that does not answer, is an empty
  package that says so; nothing waits on it.
- **Concerns (K2).** `ConcernRaised` (`source`, `what`, `severity`) is
  a child's live signal about the part above it: an application or
  `hale dna concern raise <source> <what…> [--severity N]` publishes
  it on the membrane (`hale-dna.concern.raised.sock`), the substrate
  journals `concern.raised <source>` (`<what> x<n> severity <s>`), and
  when one source has raised it `concern_threshold` times (3) it
  becomes a knowledge proposal by that source bound to its parent
  path — a concern by the tower rule — through `propose_knowledge`,
  with `concern.proposed <source>` naming the digest, once. A source
  with no parent (no `/`) is refused with `knowledge.refused
  concern:<source>`.
- **Projections and ranking (K3).** The tail also projects the
  record's `structure.observed` rows (init's loci, topics, bindings,
  effect classes and claims) into the store by kind and name, the
  latest row winning, and its `pressure.raised` and `concern.raised`
  rows into signals counted per (kind, source, what) with the last
  row's seq: `GET /structure` (counts by kind, the loci and topic
  names) and `GET /signals` answer them, so the graph has what ideas
  bind to and what the fleet is saying. Retrieval is by binding and
  provenance first: the bounded set is the accepted ideas bound to
  the target or above it, and nothing outside it is retrieved. Inside
  it, a query — the objective, which the client sends URL-encoded as
  `&query=` and the substrate passes from the ask — ranks by
  similarity so the budget keeps the most relevant
  (`KnowledgeStore.ranked_ids(target, budget, query_vec)`; the package
  says `ranked: true`; no query is ratification order). The embedding
  is lexical — `embed_text`: a hashed bag of words in 64 dimensions,
  normalized, deterministic, rendered as a pgvector literal — so `Pq`
  ranks with `<=>` over a `vector(64)` column (the store creates the
  `vector` extension at open and says so when the Postgres has none)
  and `Mem` with the same `cosine`; a hosted embedder is the same
  shape later.
- **The application side (K4).** An application declares the wire
  fact itself — a type of the membrane's shape (`source`, `what`,
  `severity`) on the subject `dna.concern.raised`, no import of the
  DNA — and publishes it when it observes something about the part
  above it. A node routes that subject for every instance it starts
  (`LOTUS_BUS_CONFIG=<node dir>/instance.bus.conf`, role connect) to
  its own socket `<clone>/.hale/node/concern.raised.sock`, which the
  `hale node` shim binds for it as an environment-configured listen
  route before the node starts; the node subscribes `ConcernRaised`
  with no source binding, and on each one appends `concern.requested
  <source>` (`source`, `what`, `severity`, `node`) to the record in
  its name and syncs. The host relays `concern.requested` rows onto
  the membrane like `intent.requested` and `review.verdict`, once
  each, and the organization journals `concern.raised`. `hale dna
  concern raise` from a clone with no membrane takes the same road.
  A route for a subject an instance never publishes is inert.
- **The learning scenario** is the acceptance (K4), in the fixture:
  the worker on the second node observes its mail backlog and raises
  the concern three times; each travels node → record → host →
  membrane → `concern.raised`; the third makes a proposal by
  `org/trio/worker` bound to `org/trio` (a concern by the tower
  rule); the Board ratifies the exact digest (`hale dna review k:…
  approve --authority board`); the knowledge service, run beside the
  organization (`hale dna knowledge`, `HALE_DNA_KNOWLEDGE_URL` in the
  organization's environment — the host forwards an operator's URL
  under `run`), tails it; and the next change to the trio consults
  the service (`knowledge.consulted m1` with the digest included),
  hands the editor the objective with the concern under it, and every
  `model.called` row of the attempt names the package and the digest.
  Something observed and ratified today informs the work done
  tomorrow, and the receipt says so.

## Backends by role

The assembly names what fills each role; `hale check` sees the wiring.

- **Record** — `Journal`: `GitJournal` (the branch), `MemJournal` (tests).
- **Membrane** — where humans see and decide: `Board` /
  `LocalHumanMembrane` over the unix sockets on one machine; the record
  itself across clones (`intent.requested`, `review.verdict` rows,
  relayed by the host); GitHub, mirrored by the host when `git config
  dna.github` names `owner/repo`: every pending mutation Review becomes
  a pull request (`github.pr`; the candidate pushed to `dna/<id>`, the
  three views as the body), every GitHub review on it becomes a
  `review.verdict` row in the reviewer's login (authority `board` when
  `dna.github.board` lists the login, else `reviewer`; each review
  once, keyed by login, commit and state), every settlement goes back
  as a comment (`github.commented`) and an approval pushes the genome.
  GitHub is a projection of the record and the record wins: a review
  whose head moved is refused here and shows as refused there.
- **Deployment** — `Deployment`: `NoDeployment` (a host expresses:
  `hale dna dev`), `ShellDeployment { command, seed }` (the command
  owns expressing and judging: `express <candidate> <seed>` returning 0
  means up and healthy, `rollback <base> <seed>` restores; the exit
  code is the observation, `expression.deployed` records it), and
  `LocalApplyDeployment` (tests). With a gateway wired, an approval
  expresses and judges in the organization's own handler and no host
  is asked; without one, `expression.restart_requested` asks the host.
- **Observation** — for a shell gateway, the command's exit; for a
  host, the window it watches; for a fleet, what its nodes report
  (below).

## The fleet

The genome's topology is the fleet plan Hale already checks
(`spec/verification.md`, "Fleet composition"), and the DNA expresses
one arrangement of it:

- **The plan describes the workspace's own services.** Plan schema
  1.2 lets an instance name a `seed` (relative to the plan) instead
  of an `artifact`; composition cuts the artifact from the seed first,
  so a candidate is checked as the fleet it would deploy. An instance
  may name the `node` that expresses it. `[dna] fleet = "<name>"` in
  `hale.toml` names which entry of `[fleets]` the DNA expresses.
- **The fleet is evidence.** Verification runs `hale fleet check --in
  <worktree> --if-declared` on every candidate (`evidence.fleet`;
  `fleet_clean`, true when the workspace declares no fleet). A change
  to one service that breaks a claim the fleet makes over all of them
  is `mutation.deny` ("candidate breaks the fleet") with the witness
  in the receipt. A candidate whose diff names a plan (`*.plan.json`)
  or the manifest (`hale.toml`) changes the fleet's shape: it is
  re-classed `topology` (`mutation.topology`) whatever it was asked
  as, and `OrgPolicy` sends it to the Board.
- **A deploy is a row.** `fleet.deploy` (entity: the Mutation, or the
  short revision for an operator's deploy) carries the plan name, the
  revision, the seed the change edits (`""` for the whole genome), the
  instances touched — every instance with a node whose seed is that
  directory — and the reason (`apply`, `rollback`, or the operator's).
  The revision is pushed to `refs/dna/revisions/<rev>` on the record's
  remote first, so every node can fetch it. Under `hale dna run` with
  `[dna] fleet` set, the host answers an application's
  `expression.restart_requested` with a deploy row; `hale dna deploy
  <revision>` and `hale dna rollback <mutation>` write the same row by
  hand. A rollback's row asks for the base; nothing is watched.
- **A node expresses.** `hale node <name> [--repo <clone>] [--fleet
  <name>] [--tick <ms>]` runs in a clone of the governed repository.
  Every tick it syncs the record and reads the latest `fleet.deploy`;
  when that names a revision it does not express, it fetches the
  revision, checks it out, and for each instance the plan assigns to
  the node that the deploy touches (or that is not up) cuts the
  artifact, builds the seed, restarts the process (cwd the clone,
  `LOTUS_OBS=1`, `HALE_DNA_NODE`, `HALE_DNA_INSTANCE`,
  `HALE_DNA_EXPRESSION`) and appends `instance.up` as `node/<name>`:
  node, instance, revision, model hash, build digest, pid. An instance
  that exits appends `instance.exited` with its code. The expression
  identity per instance is therefore reported by the node that
  expresses it and joined to the plan by instance id. The node decides
  nothing. `.hale/node/<name>/<instance>.pid` is the only local state.
- **The window is over every touched instance.** After an apply's
  deploy row the host waits for every touched instance's `instance.up`
  at the revision (180 s to settle), then for the observation window
  with no `instance.exited` at the revision among them. All up and
  none exited: `Observation healthy` with the model hash the instances
  reported. One exited: `expression.crashed` naming the instance and
  its node, `Observation crashed` with the same detail, and the
  organization rolls back — a rollback deploy row touching the same
  instances, which every node answers. Never up: `expression.crashed`
  "never expressed by …" and `Observation build_failed`.
- **`hale dna fleet`** renders what the fleet expresses from the
  record: every instance of the plan, its node, whether it is up, the
  revision and model hash it last came up at, and the last deploy.

## The host is a Hale program

Everything `hale dna` does beside the compiler that is DNA behaviour
rather than manifest or scaffolding is `dna/host`, a Hale seed
embedded in the toolchain beside the core, the membrane client and
the surface, built once into the toolchain cache. `hale dna <verb>`
resolves the project from the manifest (the root, the application's
seed, the fleet and its plan under `[dna]`) and execs the host:
`host <verb> <root> <seed> <fleet> <plan> …`, with `HALE_BIN` (the
toolchain), `HALE_DNA_MEMBRANE` (the membrane client's binary) and
`HALE_DNA_TOOLCHAIN` in the environment. The host owns the
projections (`status`, `history`, `review`, `board`, `report`,
`pressure`, `fleet`), the writers (`ask`, a verdict, `pressure raise`,
`sync`, `deploy`, `rollback`, `github sync`), the supervision (`run`,
`dev`) and the node agent (`hale node`). It reads the record through
the core's `GitJournal`, appends in a person's or a node's name with
the ref compare-and-swapped, and starts every child — the
organization, the application, iris, an instance — through `sh`,
detached, with its pid, its exit code and its log as files under
`.hale/dna/` or `.hale/node/<name>/`, echoing the log to the terminal
a tick at a time. The compiler keeps `init` / `new` / `upgrade` (they
embed the sources), `hale fleet check` and the plan schema, and the
exec shims. The host decides nothing.

## The surface

`hale dna ui [project] [--port N]` serves the DNA surface in a browser
from the record alone: a Hale program (`dna/ui`, embedded in the
toolchain like the core, the host and the membrane client, built once
into the toolchain cache) that answers every request by running one
offline verb of `hale dna` in the project root and returning what it printed
— the status projection (`/api/status`), the Board's queue
(`/api/board`), the pending Reviews and one Review's three views
(`/api/reviews`, `/api/review/<id>`), the fleet (`/api/fleet`), the
history (`/api/history[/<entity>]`), pressure (`/api/pressure`). A
verdict (`POST /api/verdict`), an intent (`POST /api/ask`) and a
pressure signal (`POST /api/pressure`) are the CLI's own verbs sent
and not waited for: onto the membrane when one is bound here, into
the record otherwise, in the name the form gives. A path segment
reaching the CLI is cleaned of separators and leading dashes, so a
request cannot name a file or a flag. The surface reads nothing
itself and decides nothing; with or without an organization up, it
shows what the CLI shows. `hale dna review <id> <verdict> --no-wait`
is the same non-blocking verdict from the terminal. Iris stays the
observer: attached to the organization's process it renders the org
as the live topology it is.

What is deliberately not here yet: a fleet-level semantic diff (a
Review's diff is the edited seed's; the deploy row names the instances
it reaches), pressure raised from services' typed metrics (`hale dna
pressure raise` is the spelling; nothing raises it for a node), and
the organization as an instance of its own plan.

