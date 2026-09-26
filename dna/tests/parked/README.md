# Parked fixtures

These drive `hale dna ui` — the old head's forms (`/api/verdict`,
`/api/task/create`, `/api/pressure`) under local names and OIDC sessions.
The record's commands are the api head's gated topics now (PR #1129,
`dna/api/commands.hl`), the ui takes no command, and it is parked until
OIDC lands on the api head (GH #989). They are not in the fixture set
`dna_native_suite` runs; they come back with #989, rewritten against
the socket and the forwarded wire.

- `b1_team_test.hl` — two people, two heads, one organism.
- `two_heads_test.hl` — two heads over one record.
- `principal_oidc_test.hl` — an OIDC session on the ui head.
