# Senses and reflexes

The **senses** are the organism's perception: one telemetry store that
every long-running part emits readings into. The **reflexes** are its
reactions that need no plan: a rule over that store that fires with no
person and no model. The operator runs both, on the private-services
tier beside memory and the nerves. They fail quietly by design: a part
that cannot serve its readings runs on unread, and a store that is down
leaves the reflexes nothing to fire on. Neither holds the record,
memory or the vault, and a reading never acts.

Beside them sit the two signals a part or a person raises by hand into
the record: **pressure** and **concerns**. Those are rows, and the
organization answers them.

## Readings

Every long-running part serves its readings as a Prometheus scrape
target (`std::metrics`, namespace `dna`), from `dna/core/senses.hl`:

| part | port | serves |
| --- | --- | --- |
| the spine (a body's host) | 9464 | `dna_pulse_seconds`, `dna_model_calls_total` |
| a node (`hale node`) | 9465 | `dna_pulse_seconds`, `dna_instance_up`, `dna_instance_since_seconds` |
| the head | 9466 | `dna_pulse_seconds` |

`HALE_DNA_SENSES_PORT` in a part's environment overrides its port (`0`
serves nothing), and `HALE_DNA_SENSES_HOST` the interface it binds
(every interface by default, so the store can reach it over the host
gateway). A port another process holds does not stop the part: it says
`the <part> cannot serve its readings on port <port>; it runs unread`
and goes on.

Every series carries `part`, and a node's carry `node` as well:

```text
dna_instance_up{part="node",node="edge-1",instance="api-0"} 0
dna_instance_since_seconds{part="node",node="edge-1",instance="api-0"} 1790558458
dna_pulse_seconds{part="node"} 1790558460
dna_model_calls_total{part="spine"} 2
```

- `dna_pulse_seconds` is each part's tick, in seconds since the epoch.
- `dna_instance_up` is 1 while an instance runs and 0 once it exited;
  `dna_instance_since_seconds` says since when.
- `dna_model_calls_total` counts the model calls whose rows the spine
  saw land.

No label is per event, per task or per revision, because a series,
once served, is never retired. A task joins the senses, its spend and
its receipts through its rows (a `model.called` row names its attempt,
and the attempt its task), not through a label.

A part never learns where its readings go.

## The store

The store is Prometheus, the organism's `senses-<id>` allocation in the
local habitat ([Memory and the record](./memory.md#the-local-habitat)).
Its service lives in the allocation's `compose.yaml`, under the
habitat's `allocations/<id>/`, never in your checkout:

```yaml
  senses-<id>:
    image: prom/prometheus:v3.5.0
    network_mode: bridge
    command: ["--config.file=/etc/prometheus/senses.yml", "--storage.tsdb.path=/prometheus", "--storage.tsdb.retention.time=7d"]
    extra_hosts:
      - "host.docker.internal:host-gateway"
    # …
```

It keeps a week, listens on 127.0.0.1 only, and its host port, in the
39000s, is chosen once and kept in the allocation's `ports.yaml`. What
it scrapes is the allocation's `senses.yml`, one job per part over the
host gateway, with `honor_labels` so a part's own labels are the
readings':

```yaml
scrape_configs:
  - job_name: spine
    honor_labels: true
    static_configs:
      - targets: ["host.docker.internal:9464"]
  # …
```

Both files are written the first time a habitat act brings one of the
organism's allocations up. `hale habitat migrate` brings the store up
with the organism's memory and nerves;

```sh
hale dna senses up [dir]
```

brings the store alone up (`docker compose … up -d --wait senses-<id>`)
and prints its read URL:

```text
HALE_DNA_SENSES_URL=http://127.0.0.1:<port>
```

`hale dna senses url [dir]` prints the same line for a store that is up,
and brings nothing up; when the store is not up it says so and names
`hale habitat migrate`. `hale dna dev` brings nothing up either: it says
whether the store is up, unless the environment names its own memory or
nerves (`HALE_DNA_MEMORY_DSN_OWNER` or `HALE_DNA_NATS_URL_OWNER`), in
which case you run your own store. Nothing stops for a store that is not
up. No part of the organism is handed the URL: the store scrapes them,
and only the reflexes read it.

## Pressure

Pressure is a signal that something needs more than it has: `<source>`
names where it is felt, and the words say what.

```sh
hale dna pressure                       # pressure raised and answered
hale dna pressure raise <source> <what…>
```

`pressure raise` writes one `pressure.requested` row, with a `request`
id of its own, and publishes nothing. A node relays the row onto the
nerves until the organization answers with the `pressure.raised` that
names it ([The nerves](./nerves.md)). Beside a live body it says
`pressure raised on <source>: <what>`; from a clone with a remote and
no body it syncs and says `pressure on <source> requested in the
record; the organization hears it there`. `hale dna ui` has a form
that does the same.

The organization counts the raises from each source in the record.
When one source has raised pressure three times (`appendage_threshold`)
it records `appendage.proposed`, *an organ for `<what>` is proposed, not
created*, and tells the Board. With `initiative` on (the default in
`dna::Dna`), that proposal becomes an `organization` Mutation of the
organization's own seed, `appendage.candidate`, verified like any change
and the Board's to decide. Nothing grows until the Board approves it.

After a Mutation is retained or rolled back, the organization re-measures
the pressure that motivated it against the fitness signals the proposal
declared (`pressure.remeasured`, [The observation
window](./heart.md#the-observation-window)).

`hale dna pressure` lists the last ten raises and every
`appendage.proposed`, `appendage.candidate` and `pressure.remeasured`
row:

```text
pressure: <n> signal(s) raised
  <seq>  <source>  <what> x<count>
  …
```

## Concerns

A concern is a part's signal about the part *above* it. Its source is a
locus path (`org/trio/worker`), and the concern is routed to that
path's parent (`org/trio`).

It enters two ways, and both end as one `concern.requested` row:

- **By hand.** `hale dna concern raise <source> <what…> [--severity N]`
  (severity 1 by default) writes the row and publishes nothing:
  `concern raised by <source>: <what>` beside a live body, or `concern
  from <source> requested in the record; the organization hears it
  there` from a clone with a remote.
- **From the application.** An application publishes its own
  `concern.raised` event, a JSON object with its `id`, `source`, `what`
  and `severity` ([The heart](./heart.md#its-events-become-readings)
  shows one). The heart lands it as a reading, `<app>/concern.raised/<id>`,
  and the spine then appends `concern.requested`, request the reading's
  entity, before the event is acknowledged. The source must be a
  `/`-path of letters, digits and `-_.`, one of whose segments is the
  application itself: an application speaks for its own parts, never
  another's, and a concern that does not is refused and recorded
  nowhere.

A node relays the row until the organization answers. The organization
admits each request once, as an execution of the `concern-escalate`
workflow (`hale dna definitions` lists it), whose one step records
`concern.raised <source>` with the `occurrence`, the `parent` it is
routed to and the execution's `task`. One request is one concern
however often it is delivered, and a second, different concern under
the same request id is refused (`concern.refused`).

**A concern that persists becomes knowledge.** When one source has
raised concerns three times (`concern_threshold`, counted from the
record, so a restart continues the count), the step proposes a
knowledge entry by that source, bound to its parent, and records
`concern.proposed <source>` naming the digest, once. The proposal is a
Review the Board decides ([The head and the face](./head.md)); ratified,
it is what the next piece of work on that part reads ([Shaping and
governing it](./shaping.md)). A source with no parent (no `/`) has
nothing to bind to: it is refused at the threshold with
`knowledge.refused concern:<source>`.

## Reflexes

The reflexes are one program, `dna/reflexes`, on the private-services
tier. It holds two things: the store's read URL
(`HALE_DNA_SENSES_URL`) and a publish-only credential on the nerves,
the `reflexes` user (`HALE_DNA_NATS_URL_REFLEXES`, its password the
vault entry `HALE_DNA_NATS_VAULT_REFLEXES` names, under
`HALE_DNA_NATS_ORG`; `hale dna nerves urls` prints all three). That
user may publish on `<org>.app.reflexes.>` and nowhere else, and no
application may publish there. To the organism the reflexes are one
more application.

**One rule is built in: `instance.down`.** Every five seconds
(`HALE_DNA_REFLEX_TICK_MS`) the program asks the store for instances
whose node reads `dna_instance_up` 0. Each fires once per outage:
`reflex.fired { id, rule, node, target, action: "restart", since,
fired_at }` on `<org>.app.reflexes.reflex.fired`, where the id names the
outage (`instance.down:<node>:<instance>:<since>`). There is no other
rule and no way to declare one.

What happens next is all rows:

1. **The heart lands the firing** as a reading,
   `reflexes/reflex.fired/<id>`, so a second publish of the same outage
   is refused as a duplicate.
2. **The node it names acts on the row.** On its tick a node reads every
   firing that names it and has no `reflex.acted` row yet, restarts that
   one instance at the latest deploy's revision (touching no other), and
   appends `reflex.acted` with the outcome: `restarted`, `already up`, or
   why not. A node restarts; any other action is refused on the row.

   ```text
   hale node edge-1: reflex instance.down:edge-1:api-0:1790558458 (instance.down on api-0): restarted
   ```

3. **A pattern becomes a concern.** The host that lands a firing reads
   the others for the same target. A second firing within ten minutes
   (`HALE_DNA_REFLEX_WINDOW_SECS`) becomes a concern, source
   `reflexes/<target>`, request `reflex:<id>`, once a window. A restart
   that did not take becomes one too (`reflex-failed:<id>`), not a quiet
   retry. From there it is a concern like any other.

The face's launcher (`hale dna head`, or `dna/face/start.sh` from a checkout) builds and starts the reflexes when it
attaches a project under the local sign-in, with that project's store
and nerves (`hale dna senses url`, `hale dna nerves urls`), and stops
them with itself. It brings nothing up: a store or nerves that is not up
leaves the face running without them (`face: no reflexes: …`, naming
`hale habitat migrate`).
Nothing else starts them.

`dna/tests/senses_reflex_test.hl` runs the whole loop on the
organism's allocations in the test file's own habitat.

## Upstream watches

The services your code calls change without asking. A **watched
upstream** notices for you, and it is a practice like any other: the
organism reads the source on the cadence the practice names and tells the
position that owns the integration when it moved. The standard pack holds
the template; propose it once per upstream:

```sh
hale dna pack apply standard --set integration=pay-changelog --set position=payments/dev \
    --set source=https://pay.example/changelog --set kind=changelog --set every=1d
hale dna pack apply standard --set integration=pay-api --set position=payments/dev \
    --set source=integrations/pay.hl --set kind=openapi --set every=1d
```

Each is one Review for the Board. It proposes the practice
`standard/upstream-watched/<integration>`, bound to the position, whose
text carries its terms:

```text
The upstream pay-api is watched for its position: … source=integrations/pay.hl kind=openapi every=1d step=source-check
```

| term | what it says |
| --- | --- |
| `source=` | for a page or a feed, its URL: `https://`, `http://` or `file://`; for `openapi`, the surface `hale api import` wrote and you committed ([Noticing when the service moves](../services/api.md#noticing-when-the-service-moves)) |
| `kind=` | `changelog` or `docs` (a page), `feed` (RSS or Atom), or `openapi` |
| `every=` | the cadence, as a schedule takes it |
| `step=` | `source-check`, the step each occurrence runs |

and beside it the idea `standard/upstream-source/<integration>`, the
source itself as knowledge bound to the same position. Once the Board
ratifies them, the practice declares its schedule (its id the practice's
name, of `source-check`, convened by its position), so `hale dna schedule`
lists it and a position nobody holds is refused as any convener is
([Schedules](./schedules.md)). An OpenAPI source takes no URL: its source
is the one the surface was imported from, and its check is `hale api
drift` on the surface. To change a watch, amend the practice; to stop
it, retire it (`retire-practice`), and its occurrences check nothing.

Each occurrence runs `source-check` with the practice as its parameters.
It reads the source and keeps only what can move for a reason: a page's
text, without its markup, scripts, styles, head or comments; a feed's
entries, a line each; an API's document, parsed and canonical (`hale api
drift --document`), summarized by its drift lines. The document is what
is compared, so a second change to an operation that already drifted is
still a move. What it is compared with is the snapshot the previous
occurrence saw, which that occurrence's outcome names. The first
occurrence is the baseline and says nothing. After that, one that finds
the same text writes nothing, and one that finds it moved lands:

- a reading, `sources/<integration>.changed/<task>`, whose payload
  carries the unified diff and names the receipt memory keeps it under;
  an API's leads with its drift lines:

  ```text
  drift Pay added create_refund
  --- standard/upstream-watched/pay-api 3a71c0d2e9b4
  +++ standard/upstream-watched/pay-api 92c69fb5be46
  @@ -30,6 +30,15 @@
  …
  +    "/refunds": {
  ```

- a concern from `<position>/<integration>`, so it is routed to the
  position, whose words name the reading and the receipt. One move is one
  concern, however often its check runs again.

A source that cannot be read (a fetch that fails, a status outside 2xx, a
drift check that cannot read its surface) fails the occurrence. One
failure is a blip and says nothing; the third in a row also lands one
reading, `sources/<integration>.unreachable/<task>`, and no concern. A
practice whose terms do not stand (a kind outside the four, say) fails
every occurrence saying why, and lands nothing.

From there the concern is a concern like any other: escalated, and
proposed as knowledge when the watch keeps raising them. When the
practice names an attached codebase (`codebase=<name>`, set with
`--set codebase=<name>` on the pack), the move also becomes a task:
see [A move becomes a task](#a-move-becomes-a-task).

### A move becomes a task

A service you depend on moved; whether your client must change is a
question about your code. When the watch's practice names an attached
codebase ([A codebase of your own](./heart.md)), the organism asks it for
you. The task is derived from what the check already landed: the reading
and its diff, and the practice that convened it. Nothing new is recorded
for it beyond the ask.

Keep the architecture note of the service in the codebase, on its default
branch, as `<watch>.md` in its knowledge directory (`--knowledge` on
`hale dna codebase attach`, default `docs/integrations/`; `<watch>` is the
last segment of the practice's name): what the client does with the
service, and where. When the service moves, the organism reads it and
proposes it as knowledge bound to the owning position (the Board ratifies
it like any knowledge). A note that changed is a fresh proposal on the
same node; one that did not is the proposal already in the record.

When the service moves, the organism files a task under the position, with
no one filing it: the reading, its diff, and the note are in the task's
brief. A leg that holds the position claims it the way it claims any task,
reads your client through the codebase's worktree, and answers one of two
ways:

- **a pull request** on the codebase's repository that updates the client;
- **a no-impact brief**, an answer that begins `NO IMPACT:` and gives its
  reasons, when nothing needs to change. No forge sees it: the organism
  puts it to a Review of its own.

Either answer comes with receipts: the note, the diff, and every file the
leg read. The leg touches nothing but the codebase's repository.

The forge's word becomes the task's verdict. A merged pull request is
**delivered**; one closed without merging is **refused**; a no-impact brief
is delivered when its Review is approved and refused otherwise. Every
comment on the pull request is kept as a receipt on the verdict row, and when the
team edited the leg's branch before merging, the task is still delivered
and the diff between the leg's last commit and the merge commit is a
receipt: what the organism learns from your edits.

## How it breaks

| symptom | cause | what to do |
| --- | --- | --- |
| `the <part> cannot serve its readings on port …; it runs unread` | another process holds the port | set `HALE_DNA_SENSES_PORT`, or free the port |
| `hale dna senses url: the organism's senses' store in the local habitat is not up …` | the store's allocation is not up | `hale habitat migrate <organism>` |
| `reflexes: the store at … did not answer (…)` | the store is down | `hale dna senses up` |
| `reflexes: the vault holds no credential …` | the reflexes' password is not drawn | `hale dna upgrade` |
| a concern that never becomes a proposal | its source has no parent | name the source as a path under its parent |
| a `source-check` occurrence failed with the result `refused` | the watch's practice has terms that do not stand | amend the practice: fix what its narrative names |
| `schedule.refused standard/upstream-watched/<name>`, `nobody holds position:<p>` | the practice's position had no holder when it was ratified | `hale dna fill <p> <holder>`, then declare it by hand: `hale dna schedule declare <name> --every <cadence> --definition source-check --convener <p> --args '{"practice": "<digest>", "name": "<name>"}'` |
