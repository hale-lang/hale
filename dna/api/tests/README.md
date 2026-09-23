# API boundary tests

The native Hale runner exercises 34 scenarios against the real HTTP service and
temporary Git Records. Fixtures use the native journal and receipt writers to
construct exact projection states, including approval before activation and
redaction with a stale local blob. They do not test domain admission or execution.

From the repository root, build the API once and run the HTTP suite:

```sh
hale build dna/api
HALE_API_BIN="$PWD/dna/api/api" \
HALE_BIN="$(command -v hale)" \
HALE_API_CONTRACT_ROOT="$PWD/dna/api/contract/v1" \
hale test dna/api/tests/read_api_test.hl
```

The paths must be absolute; `HALE_API_BIN` can select a binary built elsewhere.
The suite does not rebuild the service. Every success/error API response is
checked against the checked-in JSON Schema using the native contract validator.
Its supported schema profile is documented in the [contract README](../contract/v1/README.md).
Missing schema or service binary fails the suite. `read_api_test.hl` executes
all 34 named HTTP cases and remains silent on success. A second native test
program exercises the Organization source loader directly.
A third native program, `definitions_api_test.hl`, calls the real API handler
with an application-owned WorkflowCatalog. It validates wire schemas, exact
large revisions and costs, list/detail pagination, provenance and catalog/Record
conflicts, unsupported and invalid providers, and authentication before capture.
It proves that capability discovery does not serialize the catalog and inspection
does not replace already-admitted executions or change project refs. The browser
suite runs `catalog/main.hl` through the real HTTP startup path.

Requirements are Hale, Git, curl, standard POSIX tools, and loopback sockets. Each
case owns its temporary Record and API process; the suite hosts a native local
identity provider. Port selection probes available candidates, and startup checks
the owned child and Record identity. HTTP requests and startup waits are bounded.
Normal cleanup reaps children; an exit watcher also kills owned children and
removes scratch files when an assertion terminates the runner abruptly.

Before creating fixtures, the runner re-executes with Git plumbing overrides,
database/store URLs, evidence keys and inherited OIDC secrets removed. Global Git
configuration, curl configuration, proxies and model discovery are disabled for
the fixtures. No Postgres, Docker, organization body, external identity provider
or model runs.

Coverage includes:

- Real compiler-derived organization instances, contracts and explicit position
  groups, separate owner maps, exact node lookup and source provenance. Dirty
  source is ignored; source commits and Record changes invalidate the combined
  snapshot. Invalid committed source never returns cached structure. OIDC
  authentication applies before organization inspection and after logout.
- Actual local vendor snapshots and dependency edits at unchanged source HEAD;
  committed vendor takes precedence over dirty local files. Unsupported source
  and dependency links, imports outside the snapshot, archive attributes that
  omit or substitute committed bytes, and subprocess deadlines exercise the
  loader's unavailable/error and cleanup paths.
- Stable Record/application identity, source head/revision and honest read-only
  capabilities; typed application/object/query/method failures without ref writes.
- Unicode, multiline text and opaque slash-bearing ids; approved Review versus
  pending/refused activation; retirement and ratification precedence.
- Snapshot-bound pagination, refresh after Record advancement, missing/corrupt
  Record errors, and identical retained reads after API restart.
- Receipt absence, content-digest mismatch, redaction, classification, withholding
  and fail-closed Ledger adoption/abandonment; malformed classifications cannot
  authorize stale local blobs. Unlinked Review text does not bypass unknown
  visibility, and typed decision outcomes survive free-text suppression.
- A local authorization-code issuer, unauthenticated/forged/unmapped refusal,
  mapped session identity, logout, session loss on restart and configuration
  failure without a trusted-local fallback.
- Optional static shell serving: exact four-asset whitelist, media types and CSP,
  traversal and source-path refusal, unsupported methods, incomplete or empty
  webroot rejection, startup-loaded assets and unchanged API-only behavior.
  The shell is public without Record data; OIDC still gates API reads, and the
  successful sign-in callback lands on the served shell, whose CSP connects to
  its own origin only.

The identity-provider fixture follows the existing principal OIDC test's direct
token-endpoint trust model. It does not test a production provider, TLS or token
signature validation. These tests do not replace the native principal/domain
tests, storage tests, browser interaction tests or complete service-stack gates.

## Focused Knowledge boundary

`knowledge_api_test.hl` injects a structural provider into the real `Api` while
using a temporary native Record. It checks auth before upstream access, ignored
browser reader/credential headers, call counts, all four routes, strict query
and nested response shapes, exact signed Int64 values, Unicode escapes,
Source/basis/snapshot consistency and sanitized upstream errors. It is a public
boundary test; native state-service projection and HTTP transport have separate
integration coverage. The fixture re-executes with inherited Git plumbing and
private store/auth settings removed before making temporary Record writes.

During recovery validation, serialize native builds/runs with
`flock /tmp/face-native-validation.lock`. Build with hard address space 2 GiB,
CPU 30 seconds and wall 40 seconds; run the focused binary with hard address
space 512 MiB, CPU 10 seconds and wall 15 seconds, with core dumps disabled.
The same run limits apply to the native contract checker and validator tests.
These checks do not prove the native HTTP client's unenforced timeout field is a
deadline; see the API README's transport prerequisites.

## Practice and Review command adapter

`commands_api_test.hl` injects a typed scripted `CommandProvider` into the real
API handler. It checks trusted Origin configuration, authentication before
provider access, the expected-principal precondition, exact query/body/header
shapes, opaque IDs, Unicode, byte limits, sanitized failures and consistent
receipt identities and state relationships. Identity changes return
`409 command_context_changed` before capability or submit calls. The ordinary
executable injects `NoCommands`, advertises no writes and retains POST 405.
Review verdict cases require distinct Review/subject IDs and the pending-state
precondition, reject abstention, and keep the request's verdict acceptance/refusal
separate from observed Review settlement and adoption. Both operations use one
lookup namespace; script-only cross-operation key conflict is conformance evidence.

`commands/main.hl` is an opt-in HTTP fixture for browser conformance. It requires
`HALE_FACE_SCRIPTED_COMMANDS=1` and takes `ROOT PORT WEBROOT`. Its provider holds
request metadata in memory and returns scripted proposal/review/adoption states
selected by `ROOT/command-mode`. It does not write domain facts or provide durable
recovery. The browser tests verify real API transport and reload recovery against
the same process; they do not prove native admission, restart recovery or the
complete administration loop. See the [browser test README](../../face/tests/README.md).

Build and run `commands_api_test.hl` as a focused native test with the same shared
lock and per-process limits above. Do not infer production command capability
from this fixture or the contract examples.
