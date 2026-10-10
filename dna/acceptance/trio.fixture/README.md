# Trio replay fixture

The catalog replays the exchanges in `tape/` without model credentials.
Each filename is the exact request key produced by `RecordedModel.fields`;
source changes require updating their affected context digests and keys.

`rows.jsonl` is the record the organization writes on a replay, one
normalized row per line, sorted, and every replay must write the same
rows (the differential row harness, `dna_recorded_fixture.rs`). A
refactor of the organization proves with it that nothing the organization
does moved. A change that means to move a row re-records it with
`HALE_DNA_TRIO_ROWS=record` and names in its PR which rows moved and why:
a row's wording, a kind or a field added, a schema version the rows
carry (`"schema"`, the topology's). Recording, like replaying, needs
memory and the nerves (`HALE_DNA_MEMORY_DSN_OWNER`,
`HALE_DNA_NATS_URL_OWNER`). Without them the fixture says so and
exercises nothing, and the rows are not compared.

The organization-growth edit (`ea6311e87bce…`) and its assessment
(`f36f4798b9e3…`) were adapted for the queued cadence scaffold: the edit
preserves the new `request_tick` call and its comments while retaining the
recorded addition of the fulfilment leader. Only that deterministic scaffold
delta and the resulting request identities changed. No fresh model call was
made; the assessment answer is unchanged, and both entries' token and cost
fields remain historical usage from the original recording.

GH #985 moved memory into the core, which changed two lines of the
generated organization: its `main.hl` no longer names a ledger or a
knowledge client, and its law's `knowledge` group names
`dna::MemoryKnowledge`. The organization-growth edit's recorded output
(`f4c54ee848de…`, was `ea6311e87bce…`) carries that same template delta
and nothing else; the plans (`72800cd31bce…`, `22daa9954a05…`), the
review (`510760d5bef1…`) and the growth assessment (`deb0c457bcdb…`)
were re-keyed for the changed context alone. Each old context digest was
checked by reproducing it from the new context with the old template
line or the original recorded edit put back. No fresh model call was
made; every answer is unchanged.

GH #986 moved the organization's facts from Unix sockets onto the
nerves, which changed four places in the generated organization's
`main.hl`: it imports pond's NATS package; it holds a `nerves`
connection (before the baseline review); it binds its fact topics to
the NATS adapter, placed `pinned`, where it bound them to sockets, with
an `on_failure` for the connection after them; and its `run()` loop
ends when the program drains. Two entries carry that file in their
context. The organization-growth edit (`24445cec5037…`, was
`f4c54ee848de…`) is re-keyed, and its recorded output carries the same
four changes and nothing else. The recorded fulfilment Leader is
untouched, and the connection sits after it, before the baseline
review, where the template puts it. The growth assessment
(`7ec59ec75a20…`, was `deb0c457bcdb…`) is re-keyed for the changed
context alone. Each old context digest was checked by reproducing it
from the new context with the four changes taken back out. No fresh
model call was made; every answer is unchanged.

GH #946 gave the generated organization's `main.hl` one more binding, a
leg's outcome over the nerves (`dna::WorkSubmit: nats::NatsAdapter { }`).
Two entries carry that file in their context. The organization-growth
edit (`3d1f819f6bdc…`, was `24445cec5037…`) is re-keyed, and its recorded
output carries the same one line and nothing else. The growth assessment
(`beeae0270a4e…`, was `7ec59ec75a20…`) is re-keyed for the changed context
alone. Each old context digest was checked by reproducing it from the new
context with the one line taken back out. No fresh model call was made;
every answer is unchanged.

GH #1091 gave the generated organization's `main.hl` one more binding, a
holder asked of the organization over the nerves
(`dna::HoldRequested: nats::NatsAdapter { }`, after
`dna::PracticeRequested`). Two entries carry that file in their context.
The organization-growth edit (`d9fd4a5d2943…`, was `3d1f819f6bdc…`) is
re-keyed, and its recorded output carries the same one line and nothing
else. The growth assessment (`0dfcfca7d96c…`, was `beeae0270a4e…`) is
re-keyed for the changed context alone. Each old context digest was
checked by reproducing it from the new context, stored as the miss's
evidence, with the one line taken back out. No fresh model call was made;
every answer is unchanged.

GH #995 made the declared purpose a proposal like any practice and gave
the generated organization its workflow catalog, which changed two places
in its `main.hl`: the substrate names `catalog: workflows()` (three lines
after `genome_seed`), and the baseline purpose Review after the nerves'
connection is gone. Two entries carry that file in their context. The
organization-growth edit (`ab0a5da386e3…`, was `d9fd4a5d2943…`) is
re-keyed, and its recorded output carries the same two changes and
nothing else. The growth assessment (`a4eaa1dce8d8…`, was `0dfcfca7d96c…`)
is re-keyed for the changed context alone. Each old context digest was
checked by reproducing it from the new context with the catalog lines
taken out and the purpose Review put back. No fresh model call was made;
every answer is unchanged.

GH #946 handed the generated organization's agent work to legs, which
changed two files. `main.hl`: the comment above the work system says
so, and the system wires `agent: dna::LegRelay { name: "legs" }` with
`agent_reconciler: dna::RelayReplay { }` where it wired
`dna::AgentPerformer` over `agent_models()`. `law.hl`: its `positions`
group names `dna::LegRelay` and `dna::RelayReplay` where it named
`dna::AgentPerformer`. The organization-growth edit (`ef1f161b80d5…`,
was `ab0a5da386e3…`) is re-keyed, and its recorded output carries the
`main.hl` change and nothing else; the growth assessment
(`65a0575ac8aa…`, was `a4eaa1dce8d8…`) is re-keyed for the changed
context alone. Three entries carry `law.hl` in their context and no
recorded output does: the two classifications of asks (`e589d77675dc…`,
was `22daa9954a05…`; `21e601a79089…`, was `72800cd31bce…`) and the
review (`cbebcc1016a6…`, was `510760d5bef1…`), re-keyed for the context
alone. Each old context digest was reproduced from the new context,
kept as a run's receipt, with the change taken back out, and each new
key was the one the miss named. No fresh model call was made; every
answer, token and cost field is unchanged, and the growth edit's output
differs from the recording by the `main.hl` change alone.

GH #1123 retired the owners map: the graph is the one org chart, so the
generated organization's `main.hl` lost its five-line `ownership:` field
(the `Owners (GH #664)` comment and `dna::Ownership { path:
"dna/org/owners" }`) and nothing else. Two entries carry that file in
their context. The organization-growth edit (`6553a025bb96…`, was
`ef1f161b80d5…`) is re-keyed, and its recorded output drops the same
five lines and nothing else; the growth assessment (`0d1b72eedeb6…`, was
`65a0575ac8aa…`) is re-keyed for the changed context alone. Each old
context digest was reproduced from the new context, kept as a run's
receipt, with the five lines put back, and each new key was the one the
miss named. No fresh model call was made; every answer, token and cost
field is unchanged.

GH #1143 moved the optimize pass onto a schedule, which changed three
places in the generated organization's `main.hl`: the `optimize_every_ms`
field and its comment go (a comment above `planned: true` says the pass
occurs on the cadence a ratified practice declares), the comment above
the `run()` loop says the schedules occur on its tick, and the loop ticks
on the wall clock (`std::time::nanos(std::time::current())`) where it
ticked on the monotonic one. Two entries carry that file in their
context. The organization-growth edit (`911a2531e52e…`, was
`6553a025bb96…`) is re-keyed, and its recorded output carries the same
three changes and nothing else; the growth assessment (`0f8fa3aab6c4…`,
was `0d1b72eedeb6…`) is re-keyed for the changed context alone. Each old
context digest was reproduced from the new context, kept as a run's
receipt, with the three changes taken back out, and each new key was the
one the miss named. No fresh model call was made; every answer, token and
cost field is unchanged.

The Board's field on `dna::Dna` was renamed from `membrane` to `board`,
which changed one line of the generated organization's `main.hl`
(`board: dna::Board { who: "board" }`). Two entries carry that file in
their context. The organization-growth edit (`aaff169d9019…`, was
`911a2531e52e…`) is re-keyed, and its recorded output carries the same
one line and nothing else; the growth assessment (`1ba00dde397f…`, was
`0f8fa3aab6c4…`) is re-keyed for the changed context alone. Each old
context digest was reproduced from the new context, kept as a run's
receipt, with the line put back, and each new key was the one the miss
named. No fresh model call was made; every answer, token and cost field
is unchanged.

GH #989 sealed the nerves' credentials, which changed one place in the
generated organization's `main.hl`: its `nerves` connection gains four
lines (a two-line comment, `user: "spine"`, and `credential:
std::secret::Credential { vault: dna::nerves_role_vault("spine") }`).
Two entries carry that file in their context. The organization-growth
edit (`0f311e0493a1…`, was `aaff169d9019…`) is re-keyed, and its
recorded output carries the same four lines and nothing else; the
growth assessment (`5e1da236ac7c…`, was `1ba00dde397f…`) is re-keyed
for the changed context alone. Each old context digest was reproduced
from the new context, kept as a run's receipt, with the four lines taken
back out, and each new key was the one the miss named. No fresh model
call was made; every answer, token and cost field is unchanged.

GH #1131 put a leg's ask for its attempt's spend on the nerves, which
changed one place in the generated organization's `main.hl`: its
`bindings` gain `dna::WorkAllowanceAsk: nats::NatsAdapter { };` after
`dna::WorkSubmit`. Two entries carry that file in their context. The
organization-growth edit (`25dcff4e759e…`, was `0f311e0493a1…`) is
re-keyed, and its recorded output carries the same line and nothing else;
the growth assessment (`860b4feb6840…`, was `5e1da236ac7c…`) is re-keyed
for the changed context alone. Each old context digest was reproduced
from the new context, kept as a run's receipt, with the line taken back
out, and each new key was the one the miss named. No fresh model call was
made; every answer, token and cost field is unchanged.

The branch that pulls knowledge to a hat by its targets changed what the
hat carries: a hat now asks for a set of targets (the Work's path, its
codebase's languages, the system for an organization change, and the
performer's position), the editor's `PRACTICES (ratified knowledge for …)`
header names the whole set, and the hat's JSON gains `targets`, which its
digest covers. Four entries carry that header or the hat in their request.
The editor's docs edit (`2f32bc2896d6…`, was `60d122ddbd49…`) and the
editor's cross-service edit (`fe9d9e6e68f8…`, was `35b9375d83e3…`) are
re-keyed for a moved `prompt_digest` (the context digests are unchanged);
the two assessments that follow them (`7cd0bddc1816…`, was
`2e5850792174…`; `cc1007e39566…`, was `c9521f468f24…`) are re-keyed for a
moved `context_digest` (the prompt digest is unchanged). Each new key was
the one the miss named, each old key was reproduced from its entry's fields
before it was replaced, and the entries kept their `role`, `grant`, `tree`
and `data_class`. No fresh model call was made; every answer, token and
cost field is unchanged. In `rows.jsonl` three rows moved, the
`knowledge.consulted` rows of `m1`, `m2` and `m3`, whose `target` now names
the set (`org/trio position:editor`; `org system:dna position:editor` for
the organization change `m2`; `org/trio position:editor`); the hat digest
in them is normalized, so no row shows it.

A position opened at run time (`hale dna position open`) gave the
generated organization's `main.hl` one more binding, the request over
the nerves (`dna::PositionOpenRequested: nats::NatsAdapter { }`, after
`dna::HoldRequested`). Two entries carry that file in their context.
The organization-growth edit (`abc3822558b3…`, was `25dcff4e759e…`) is
re-keyed, and its recorded output carries the same one line and nothing
else. The growth assessment (`9792e47cd54c…`, was `860b4feb6840…`) is
re-keyed for the changed context alone. Each old context digest was
checked by reproducing it from the new context, stored as the miss's
evidence, with the one line taken back out. No fresh model call was
made; every answer is unchanged.
