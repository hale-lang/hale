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
