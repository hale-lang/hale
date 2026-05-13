# bridge/

The ferryman flow. Carries devs across the boundary between
unfamiliar code and seen-as-lotus.

**What goes here.** A walkthrough of pointing ferryman at a
codebase, reading the yaml it emits, the agent-enrichment
protocol, the recognition reports, and — eventually — the
path from enriched yaml to a mechanical Aperio rewrite. The
larger narrative of why this is the front door into the
language.

**Reader.** A dev with an existing system they want to see as
lotus, then migrate from. Go is the v0 target; the general
domain — orgs, pipelines, anything with emergent structure —
sits on the horizon.

**Source of truth.** `../../apps/ferryman/` is the binary;
its README is the authoritative how-to. This track wraps that
in the larger story.

**Status.** Empty scaffold. Content arrives in purpose-driven
sessions.

**Worth reading first** (when this track starts being filled):

- `../../apps/ferryman/README.md` — pipeline, CLI surface,
  conventions, smoke testing.
- `../../notes/codebase-onboarding-design.md` — the primary
  design plan for the codebase-onboarder arc.
- `../../notes/agent-onboarding/ferryman-enrichment-protocol.md` —
  long-form version of the agent-enrichment protocol that
  ferryman ships as `PROMPT.md`.
- `../../notes/aperio-types-vs-loci.md` — the three-tower
  agreement rule that drives recognition.
- `../../notes/onboarding-shape-rules.md` — the Agent /
  Entity / Shape noun categories used during enrichment.
