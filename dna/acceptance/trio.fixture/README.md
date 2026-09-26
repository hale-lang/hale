# Trio replay fixture

The catalog replays the exchanges in `tape/` without model credentials.
Each filename is the exact request key produced by `RecordedModel.fields`;
source changes require updating their affected context digests and keys.

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
