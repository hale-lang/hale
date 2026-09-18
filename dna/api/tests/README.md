# Read-only API integration tests

The native Hale runner exercises 29 scenarios against the real HTTP service and
temporary Git Records. Fixtures use the native journal and receipt writers to
construct exact projection states, including approval before activation and
redaction with a stale local blob. They do not test domain admission or execution.

From the repository root, build the API once and run the suite:

```sh
hale build dna/api
HALE_API_BIN="$PWD/dna/api/api" \
HALE_API_CONTRACT_ROOT="$PWD/dna/api/contract/v1" \
hale test dna/api/tests
```

Both paths must be absolute; `HALE_API_BIN` can select a binary built elsewhere.
The suite does not rebuild the service. Every success/error API response is
checked against the checked-in JSON Schema using the native contract validator.
Its supported schema profile is documented in the [contract README](../contract/v1/README.md).
Missing schema or service binary fails the suite. `hale test` reports one test
program; that program executes all 29 named cases and remains silent on success.

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
- Optional static shell serving: exact asset whitelist, media types and CSP,
  traversal and source-path refusal, unsupported methods, incomplete or empty
  webroot rejection, startup-loaded assets and unchanged API-only behavior.
  The shell is public without Record data; OIDC still gates API reads, and the
  successful sign-in callback lands on the served shell.

The identity-provider fixture follows the existing principal OIDC test's direct
token-endpoint trust model. It does not test a production provider, TLS or token
signature validation. These tests do not replace the native principal/domain
tests, storage tests, browser interaction tests or complete service-stack gates.
