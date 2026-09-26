# Native Practice and Review command head

This source-built Hale head composes `operations::GovernanceCommands` into the
existing loopback API. The ordinary `dna/api` executable remains read-only.
Admission and recovery live in the shared native service; the adapter only maps
typed API values and preserves the service's exact Record evidence.
The frozen policy implementation is shared in `dna/operations/governance_policy.hl`.

The supported deployment is one local Record with a DNA body and host built
from this same source revision, including the host that relays its commands.
An older body cannot supply command-specific decisions or recovery. The host
consumes admitted governance facts through the normal native
path. A recorded command does not prove that the body has progressed it. Run the
body and host separately as usual; this head does not fabricate outcomes, relay
unsigned facts around the host, or make a multi-clone consistency promise.

This initial composition requires Record-only receipt visibility. Once a Record
has `ledger.adopted`, new commands fail closed because the owning Ledger may hold
receipt restrictions that this reader cannot prove absent. Governance facts still
route to Record, but that alone does not establish routing-1 command support.
Supporting it requires an owning-service eligibility reader and a decision boundary
that protects its captured Ledger visibility as well as the Record predecessor.
Raw local receipt blobs are never a fallback for that missing policy evidence.
Authorized recovery of this principal's existing command receipts remains
available through the native service's safe outcome projection.

```sh
hale build dna/api/practice_review
HALE_DNA_COMMAND_POLICY=/path/to/authority.json \
  dna/face/start.sh /path/to/project \
  --api dna/api/practice_review/practice_review --port 8792
```

The binary accepts the launcher's existing `PROJECT PORT WEBROOT` arguments;
direct invocation can omit `WEBROOT` for API-only use. The explicit policy path
comes only from `HALE_DNA_COMMAND_POLICY`, which is required.

The commands are the api binding's (GH #1104 piece 5): this seed
declares its own `main locus` with the `api:` entry and a wrapper
(`ReviewCommands`) subscribing the head's command topics under the
same gates over the shared admission (`api::CommandAdmission`), since
the surface is the entrypoint seed's own loci and an imported main's
bindings are inert. The socket is one per record under
`$XDG_RUNTIME_DIR/hale/dna/`, named in `/capabilities` (`api.socket`);
`LOTUS_API` overrides it. `hale check --dump-api dna/api/practice_review`
lists the twelve as `api::…`. An application with
its own Workflow catalog can compose `PracticeReviewCommands` and call
`api::serve_with_commands` with that catalog instead of this head's
`NoWorkflowCatalog`. Retain the authority, native service and adapter for the
entire server lifetime.

## Explicit authority

The required policy file is trusted startup configuration. Use the application's
Record genesis ID in `application_id`; the head rejects a policy for any other
application. No name, role or grant is inferred from `USER`.

```json
{
  "format": "dna.practice-review-authority/1",
  "application_id": "RECORD-GENESIS-ID",
  "grants": [
    {
      "mode": "local",
      "name": "alice",
      "authority": "board",
      "practice_propose": true,
      "review_verdict": false,
      "recover": true
    },
    {
      "mode": "oidc",
      "name": "bob",
      "authority": "board",
      "practice_propose": false,
      "review_verdict": true,
      "recover": true
    }
  ]
}
```

The policy is closed JSON, at most 64 KiB and 128 grants. Each exact `(mode,name)`
pair may occur once. Unknown fields, duplicate keys, ambiguous grants, unsupported
authorities and malformed Unicode are refused. An empty grants array is valid and
denies everyone. Supported authorities are the native `board`, `maintainer`,
`leader`, `supervisor` and `reviewer` roles. A grant permits the specified operation
to reach the native service; it does not override the service's role, subject,
independence or eligibility checks.

Authentication uses the existing API startup path. Local mode resolves the trusted
local actor; OIDC resolves the authenticated subject through `dna.oidc.member`.
The policy names that resolved identity, not an email or a caller-supplied role.
Local and OIDC identities with the same name are distinct. The example shows both
forms; the configured principal mode determines which is active. Missing mappings
deny command access. Receipt lookup requires `recover: true` for the current
principal and is still scoped to that principal's own command identity. It can be
granted without either write permission.

The exact policy document and captured `dna.trust` mode form the recorded authority
basis. They remain fixed for this provider lifetime. `dna.trust=signed` requests a
signed Record commit; unsupported trust modes and unreadable configuration fail
startup. Git remains responsible for the configured signing key and verification.
Changing the policy, authentication or signing configuration requires stopping and
restarting this deployment. Record head CAS protects admission against competing
Record changes; it does not fence files or Git configuration. This head does not
claim hot policy reload or atomic authority revocation.

## Raising work needs no policy

This head also supports `dna.task.create@1`, the face's `hale dna ask`. It
takes no grant from either policy file: the authenticated principal is the
authority, as for the CLI, and the organism judges whether the position is this
organization's to admit. Recovery of an ask by its request key follows the
usual rule, so a principal with `recover: true` in `HALE_DNA_COMMAND_POLICY`
can look up a key that never landed and read `command_not_found` instead of a
denial. See the [API README](../README.md#raising-work) for the wire shape and
the row it writes.

## The same operations from the CLI

The source-built host uses this same policy and native command service for
Practice replacements and named org-wide Practice Review verdicts. Set
`HALE_DNA_COMMAND_POLICY` explicitly. The embedded installed-CLI package inventory
is not updated by this source-built profile.

```sh
hale build dna/host
export HALE_DNA_COMMAND_POLICY=/path/to/authority.json
dna/host/host practice /path/to/project . - - propose practice-name \
  --supersedes PREDECESSOR-DIGEST --text 'Replacement text' \
  --because 'Reason for the change' --request-id replacement-1 --no-wait
dna/host/host verdict /path/to/project . - - REVIEW-ID approve \
  --digest CANDIDATE-DIGEST --request-id verdict-1 --no-wait
dna/host/host practice /path/to/project . - - lookup --request-id replacement-1
```

`practice lookup --request-id ID` recovers either supported operation in the same
application/principal namespace. Once packaging includes these operations, the
corresponding public syntax is `hale dna practice ...` and
`hale dna review REVIEW-ID approve ...`.

The CLI principal mode is always `local`, using the exact nonempty `USER` identity.
It needs its own explicit local policy grant even when the web head uses OIDC.
`--as` can only repeat that actual identity. For verdicts, `--authority` can only
repeat the role already assigned by policy; neither flag grants authority.
Each invocation captures the current policy, and lookup enforces current recovery
permission. Reusing an ID for different typed content conflicts in the native
service. Preserve the exact text, rationale and subject when retrying a POST-like
submission; use lookup when only recovering an uncertain outcome.

An omitted request ID is generated from time, process ID and a monotonic clock.
The CLI prints it before admission and repeats it in the receipt. `--no-wait`
returns after admission. When a local body is present, the default waits for a
native answer by reading that same request identity. Without a local body the
command can still be recorded, and the CLI returns its current receipt. Waiting
does not republish a trigger; a timeout does not imply failure or invent success.
Verdict acceptance, Review settlement and Practice activation are printed
separately.

With a policy configured, supported replacements and Practice Review verdicts
always use shared admission. Creation of a new Practice, the `design` batch helper
and unrelated Review profiles keep their existing CLI paths. Those legacy paths
reject `--request-id` rather than pretending to offer this recovery contract.

## Optional Organization evidence and recovery

`HALE_DNA_ORGANIZATION_POLICY` optionally names a separate, application-bound `dna.organization-authority/1` policy (1..65536 bytes). Its grants have `mode`, `name`, `authority`, `organization_propose`, `organization_review`, and `recover` fields. They never inherit Practice permissions. The same named authority is injected into native recovery and immutable source candidate reads; `recover:true` can permit an independent reviewer without proposal permission. Invalid configured source policy refuses startup.

This head currently fixes `organization_enabled:false` and `organization_reviews_enabled:false`. Its new Organization capability profiles therefore report unavailable writes even if the policy authorizes them; no environment toggle opens premature source publication. Exact source receipt lookup remains possible, and reports proposal, Review, application and restart handoff separately. Activation remains unknown. Existing Practice/Review request and receipt shapes and the shared request-ID namespace are preserved.

When the independent Organization policy is configured, this local head also binds the shared native `OrganizationRuntime { root }` reader to source status. The source recovery/read grant is checked before inspecting the process. An association is available only when the API shares the Host project and PID namespace and the native source, launch, artifacts, lease and exact process identity still match. Other deployments return unavailable; this is not a remote process probe. Without a configured source policy, the runtime facet remains unavailable. Historical launch/observation facts and this fresh association remain separate, and source writes stay disabled.
