# Schedules

A **schedule** says when a workflow runs and who convenes it, and
nothing else. It points at a definition in the workflow catalog, never
at a sentence to ask, and each occurrence is one execution of that
definition, admitted like any other. The organization owns its
schedules (the `Scheduler` in `dna/core/scheduler.hl`): a practice the
Board ratifies declares one, and the Board or the convener's holder may
ask for one. A schedule fails
loudly, as rows: an occurrence that passes while the organism is down
is `schedule.missed`, one that arrives while the last execution is
still open is `schedule.skipped`, and a declaration that cannot stand
is `schedule.refused`, with the reason. A schedule holds no intent
text, no claim and no ask of its own.

## See them

```text
$ hale dna schedule
schedules: none declared (a ratified practice declares one, or `hale dna schedule declare`)
```

That is a fresh project. Once schedules are declared, each is one line
as the record has it:

```text
schedules: <n>
  <id> [live|paused] every <n>ms|cron `<expr>` (UTC) — <definition>, convened by <position> · occurred N, skipped N, missed N · last <task>
```

*occurred* counts the executions admitted under the schedule's key,
and *last* names the latest one's task. The listing reads the record
alone, so it works from any clone, with or without an organism
running.

## Ask for one

```sh
hale dna schedule declare nightly --cron "0 2 * * *" --definition ask-edit --convener editor --args '{"objective": "reconcile the records of the day"}'
```

| flag | what it says |
| --- | --- |
| `<id>` | the schedule's name: no `@`, no space |
| `--every <n>ms\|s\|m\|h\|d` | an interval (`90s`, `15m`, `1d`) |
| `--cron <expr>` | five fields, minute hour day-of-month month day-of-week, in UTC: `*`, `a`, `a-b`, `*/n`, `a-b/n` and lists of them |
| `--definition <id>` | a definition in the catalog (`hale dna definitions` lists them) |
| `--convener <position>` | who convenes it; a bare name is taken as `position:<name>` |
| `--args <json>` | the execution's inputs, a JSON object (default `{}`) |
| `--as <who>` | whose request it is (default: you) |

Give exactly one of `--every` and `--cron`. The verb is a request, not
a decree: it writes a `schedule.requested` row (the schedule's terms,
the convener as `position:<name>`, and who asked) under a request id
of its own. A node relays the row onto the nerves. Beside a running
organism the verb then waits up to a minute for the answer,
`schedule.answered` under the same id; from a clone with none, it syncs
the request and says so:

```text
schedule nightly asked of the organization in the record by <who> (<request>); it declares it when it hears it
```

The organization's connection to the nerves hands each message it
receives to the topic its subject names, so the relayed request reaches
the organization's `ScheduleRequested` subscription without any entry
in the `bindings { }` of `dna/org/main.hl`.

## How the organization answers

An organization that receives a request answers it once, under the
request's id: `schedule.answered` with `declared` true, or false and
`why`. The verb prints the answer:

```text
schedule nightly declared: ask-edit on cron `0 2 * * *` (UTC), convened by position:editor
```

or `schedule declare: refused: <why>`.

**Who may ask.** Declaring through a request is the Board's, or the
convener's own holder's. Once memory names who holds the Board, a
requester who holds neither the Board nor the convener is answered
`declared: false`, and nothing is declared. While nobody holds the
Board, or no memory is named, the requester declares, as a Review's
fallback decides.

**What a declaration must be.** Whether it comes from a request or a
practice, a declaration is refused, as a `schedule.refused` row with
the reason, when:

- the id is empty or has `@` or a space;
- it names neither an interval nor a cron, or both;
- the cron is malformed (checked when it is declared, never when it
  would first fire);
- the convener is not `position:<name>`, or cannot reach what the
  definition writes;
- the definition is not in the catalog, the catalog would refuse its
  expansion, or it has a step the organization does not perform;
- the inputs are not a JSON object naming every input the definition
  takes. `hale dna definitions` says what each takes:

```text
ask-edit@1  an ask: one change, prepared for review
  0. e  record
  occurs: skip while open, takes objective
```

Declaring the same schedule again, unchanged, writes nothing.
Declaring it with other terms writes a new `schedule.declared` row,
and the new terms hold from then on.

## Conveners

The convener is the position that runs the execution: the admission is
its owner's, and `workflow.admitted` names it as `convener`.

- The organism's own positions, `leader` and `editor`, reach every
  store the organization's workflows write, and convene with no holder.
- Any other position convenes through whoever holds it (a person, or
  an organization's members), as memory's graph says. With no memory
  named, a graph position is refused: nothing is assumed.
- A position nobody holds convenes nothing. The refusal says so and
  names the fix, and the row carries the hole:

```text
nobody holds position:<p>: it convenes nothing until someone does (`hale dna fill <p> <holder>`)
```

Declaring also puts the rhythm in the graph: a `definition:<id>` node,
and a `convenes` edge from the position to it carrying the cadence
(`every 1d`, `every 90s`, `cron <expr>`). `hale dna show processes`
ends with one line per rhythm, `<position> convenes <definition>
<cadence>` ([Memory and the record](./memory.md)).

## Occurrences

The organization ticks on the wall clock. An occurrence is named by
its time: an interval's step from the epoch, or a cron's matching
minute in UTC, evaluated once a minute. A tick in an occurrence's
period admits it as an execution of the definition's newest revision,
under the ask id `sched:<id>@<time>`, with `--args` as its inputs.
That key is the idempotence: an occurrence asked twice, or asked again
after a restart, is one execution, because the schedule's state is
rebuilt from the record.

| what happened | the row |
| --- | --- |
| the occurrence was admitted | `workflow.admitted`, its request `sched:<id>@<time>` |
| the last execution was still open, and the definition does not overlap | `schedule.skipped <id> {occurrence, task}` |
| occurrences passed with no tick (the organism was down) | one `schedule.missed <id> {first, last, count, why}` per run of them |
| the admission was refused | `schedule.refused <id> {occurrence, why}` |

Missed occurrences are not run late. What happens while an execution
is open is the definition's: `skip` by default, or `overlap` to admit
the new one beside it ([The spine](./spine.md) has the catalog). A
declaration names the occurrence it was made at, and nothing at or
before it is due.

## Pause and resume

```sh
hale dna schedule pause nightly
hale dna schedule resume nightly
```

Each is a row in your name (`schedule.paused`, `schedule.resumed`),
written from any clone and read by the organism at its next tick:

```text
schedule nightly paused (schedule.paused; nothing occurs until it is resumed, and nothing is missed)
schedule nightly resumed (schedule.resumed)
```

A paused schedule is not due: its occurrences pass unfired and
unmissed, and a resume goes on from the next one. A resume the
organism reads at a restart counts misses only from when it was
resumed. Pausing a paused schedule, or resuming a live one, says so
and writes nothing.

## A practice carries a schedule: the optimize pass

A ratified practice can declare a schedule. The toolchain seeds one:
`operating/optimize-cadence`, whose receipt carries the optimize
pass's cadence. It sits in a fresh project's record, waiting for the
Board with the other seeded practices:

```text
$ hale dna review
…
  operating — 7 seeded practice(s), each its own Review:
…
      k:fffbc971da28 — ratify the operating practice `operating/optimize-cadence`: walk the machinery on a cadence: the optimize pass is an execution of op…
```

```text
$ git cat-file -p refs/dna/receipts/fffbc971da28fafdbd6f3aa6ca5bcd4eb3e2e3626ed8140ef687f896d3540e50
{"author":"org","kind":"practice","name":"operating/optimize-cadence","provenance":"design","schedule":"{\"id\": \"optimize\", \"every_ms\": 86400000, \"definition\": \"optimize-walk\", \"args\": \"{}\", \"convener\": \"position:leader\"}","target":"org","text":"walk the machinery on a cadence: the optimize pass is an execution of optimize-walk, convened by the leader once a day, which proposes one small change or records that the state is clean. The cadence is this practice's; a different one is an amendment the Board ratifies.","toolchain":"0.21.0"}
```

The id is the receipt's digest, so it differs from one toolchain to
the next. When the Board ratifies it (`hale dna review <id> approve`,
or the whole family with `hale dna review operating approve`), its
ratify step declares the `optimize` schedule: every day,
`optimize-walk`, convened by the leader. The ratification stands even
when the declaration is refused; the step says `its schedule was
refused (<why>)`, and the refusal is its own `schedule.refused` row.

Each occurrence is one pass. Its one step, `walk`, asks the budget
first (on a spent window it is `optimize.refused`, and the pass waits
for the next occurrence), then reads the record's structural signals
and asks the leader to walk the machinery, not the work. The leader
answers with one small proposal or none, and `org.reviewed` records
either. A different cadence is an amendment to the practice, which the
Board ratifies ([Shaping and governing it](./shaping.md)).

## The rows

| row | written by | what it says |
| --- | --- | --- |
| `schedule.requested` / `schedule.answered` | `schedule declare` / the organization | a schedule asked for; its answer under the request's id: `declared`, `why` |
| `schedule.declared` | the organization | `every_ms`, `cron`, `definition`, `args`, `convener`, `from` |
| `schedule.refused` | the organization | a declaration that would not stand (`why`, and `hole` for a convener nobody holds), or an occurrence whose admission was refused |
| `schedule.skipped` / `schedule.missed` | the organization | an occurrence not admitted while the last execution is open; occurrences whose period passed with no tick |
| `schedule.paused` / `schedule.resumed` | you | paused and resumed by hand |

## How it breaks

- **`schedule declare: refused: <why>`**, or a `schedule.refused`
  row: the reason names what to fix; the list above has every case.
- **`nobody holds position:<p>`**: fill the position
  (`hale dna fill <p> <holder>`). A refused declaration is not tried
  again on its own once the hole is filled.
- **``who holds position:<p> is the graph's, and no memory is named to ask``**:
  a graph convener needs memory; `hale dna memory migrate` prints the
  DSNs ([Memory and the record](./memory.md)).
- **`asked for the schedule, but the organization journaled no answer for <request> in time`**:
  no organization answered within a minute. The request stays in the
  record, and a running organization declares it when it hears it.
- **`missed N`** in the listing: the organism was down through those
  occurrences. They are not run late; the next one runs on time.
