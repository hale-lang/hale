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
