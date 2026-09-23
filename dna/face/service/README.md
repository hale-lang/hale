# Generic Hale application head

This native Hale library serves the `hale.application.v1` profile under
`/api/hale/v1/applications`. An application explicitly supplies one
`ApplicationProvider`; the face adapts its typed state and enum-control operation.
No DNA Record, organization, position, database credential in the browser, CLI
subprocess dispatch, or observer-to-application identity inference is involved.

The head owns transport checks and response validation. The application owns
authorization, current incarnation, revision checks, state, immutable receipts
and recovery. The [intake-control example](../../../iris/examples/intake-control/README.md)
is a separate application-owned proof adapter; other providers must establish
their own completion and persistence guarantees.

## Composition

Import `dna/face/service` from the application entry point and call:

```hale,fragment
let provider = MyApplicationProvider { };
service::serve(port, service::Actor { name: "operator" }, provider, webroot);
```

The exact signature is `serve(port: Int, actor: Actor, provider:
ApplicationProvider, webroot: String)`. The provider is a named, explicit
binding. The head captures its 64-character lowercase hexadecimal application
identity at startup and refuses a replacement identity. It binds only
`127.0.0.1`. Its readiness marker is `iris-application:listening`, emitted after
the native socket listens. An empty `webroot` selects API-only mode.

The first profile is explicitly **trusted local**. `actor.mode` must be
`trusted_local`, with a startup-configured name matching
`[A-Za-z0-9._:-]{1,128}`.
The request body never selects the actor. Its expected principal is compared
with that configured identity before operation dispatch. Capabilities remain
advisory: providers recheck current access for both commands and recovery.
This does not provide hosted sessions, OIDC or remote multi-user authentication.

With a webroot, the head loads the ten fixed browser assets once, injects the
constant `data-iris-profile="application"` on the HTML element and serves the
face's shell. Asset reads precede provider access. The shell has no
runtime view; inspect the running application with `hale iris`. Restart the
head after changing assets or configuration.

## Wire contract

The [JSON Schema](contract/v1/schema.json), [OpenAPI document](contract/v1/openapi.json)
and [illustrative shape fixtures](contract/v1/fixtures.json) describe the first
profile. Every success reports `api_version:"hale.v1"`,
`profile:"hale.application.v1"`, the configured principal and typed `data`.
Errors carry fixed, sanitized codes and explanations. No DNA `source` is added.

| Route | Meaning |
| --- | --- |
| `GET /applications` | One explicitly registered application descriptor |
| `GET /applications/{app}/state` | One captured application-owned state read |
| `GET /applications/{app}/capabilities` | Registered versioned enum controls and current availability/authority |
| `POST /applications/{app}/commands` | One exact, revision- and incarnation-guarded operation |
| `GET /applications/{app}/commands?request_id=…` | Recovery under the current actor and stable application identity |

Paths in the table are relative to `/api/hale/v1`. State distinguishes committed
control revision/value from observed effective revision/value and work count.
Empty effective fields mean unavailable. `online` describes application-owned
heartbeat freshness. Activity count and observation tick are exact decimal
strings, scoped as defined by the application; tick is not wall-clock time.

All requests require the exact configured `Host`; a supplied `Origin` must match
the loopback origin. POST additionally requires exactly one matching `Origin`,
`Content-Type: application/json`, `X-Iris-Command: 1`, and canonical decimal
`Content-Length` equal to the actual 1–8192 body bytes. Duplicate security or
framing headers and `Transfer-Encoding` are rejected. Browser fetch and ordinary
curl requests supply Content-Length; direct API clients must also send the
Origin and intent header explicitly. These checks protect the local boundary;
they are not a substitute for hosted authentication.

Mutation objects are closed. Duplicate or escaped keys, unknown fields, NUL,
invalid UTF-8, wrong types and invalid decimal values fail before submission.
The parser and Unicode decoder are reused by single-file import from the pure
`dna/operations/organization_json.hl` utility. The shell similarly imports only
the pure `dna/api/web.hl` file. Neither import brings in DNA domain services or
the operations seed. Neutral package extraction is a separate ownership task.

Application and incarnation IDs are 64 lowercase hexadecimal characters.
Principal names, request IDs, operation/version, target IDs and enum values match
`[A-Za-z0-9._:-]{1,128}`. Revisions, counters and ticks are canonical nonnegative
Int64 decimal strings. Labels and application names are nonempty UTF-8, at most
512 bytes, with no control characters. At most 16 operations and 32 distinct
choices per operation are exposed; serialized choices are bounded to 8192 bytes.
These byte and cross-field checks are imperative in addition to JSON Schema.

## Runtime association

State may include `runtime: {profile:"hale.iris.process.v1", process_key, pid}`
while online. The provider explicitly binds this evidence to the returned
application and incarnation; omission means no established association. The
64-hex process key matches the native observer's optional `process_key`, and PID
is a canonical decimal string. Neither field grants command authority.

The local Linux reader in `iris/process_identity` hashes the kernel boot ID,
PID, process start tick, PID namespace device/inode and observation segment
device/inode. It requires readable `/proc`, the segment in `/dev/shm`, and
`/usr/bin/stat`. Missing facilities leave observation unassociated. This is a
same-host, same-PID-namespace profile; it does not establish a distributed or
hosted identity contract. The collector captures matching identities around
attachment, then checks the process start tick while serving observations.

The browser refreshes the application-owned association while observing and
links only an exact live process match. Application/incarnation/principal
replacement requires returning to Application and reopening Runtime. Missing,
stale or inaccessible evidence removes the control link. Opening controls reads
current capabilities and state again; normal command preconditions still apply.
Legacy snapshots and providers remain usable without association metadata.

## Decisions and recovery

The provider's deduplication namespace is stable application + actual actor +
request ID, across operation types and application incarnations. After current
access checks, an equivalent existing request is recovered before applying
incarnation, revision or availability checks to new admissions. A conflicting
reuse returns `409 request_conflict` without disclosing the existing payload.
Unknown operations are rejected before provider submission.

`succeeded` proves the desired configuration was committed. A changing command
increments the expected revision once; a new same-value command first checks
the expected revision and succeeds unchanged. Current effective behavior is a
separate state read. A retained refusal has `changed:false` and a reason:
POST returns 409 with both error and receipt, while lookup returns 200 with
the same refused receipt. Unestablished storage or a contradictory provider
receipt returns 503. Error-only `404 command_not_found` is authoritative absence
for that lookup, never permission for automatic resubmission.

Fingerprint framing is UTF-8 with LF separators and **no trailing LF**, in this
order: profile, actual principal mode, actual principal name, application ID,
request ID, operation, operation version, target kind, target ID, incarnation ID,
expected revision, desired value. The fingerprint is `sha256:` followed by the
lowercase SHA-256 digest. The head recomputes this fingerprint and verifies
actor/application/request attribution, exact submitted intent and result
invariants before emitting a receipt. A historical lookup keeps its original
incarnation; it is not compared to the current runtime instance.

Providers must atomically retain request decisions and configuration, enforce
current authority, fence old workers, preserve receipts across head/target
restart, and refuse corrupt or incompatible state. This profile advertises
`application_store_lifetime` retention: no silent eviction or re-admission after
capacity exhaustion. Recreating storage must establish a new application
identity. The head keeps no authoritative request table.

## Focused verification

`tests/boundary_test.hl` exercises strict requests, header/framing checks,
principal fencing, unknown operations, exact numbers, sanitized failures,
receipt validation and public assets with a scripted provider. It is HTTP
adapter conformance, not evidence of real configuration effects or durability.
The intake application owns separate integration and restart proofs. Browser
confirmation/recovery behavior is tested in the face's browser suite. Use the shared
native-validation lock and bounded native build/runtime limits for local gates;
there is no compiler or DNA engine suite dependency here.
