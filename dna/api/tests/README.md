# Read-only API integration tests

These tests launch the real Hale HTTP service over a temporary Git Record. They
write the existing native journal/receipt format directly to construct exact
projection states, including approval before activation and redaction with a
stale local blob. They do not claim to test domain admission or execution.

Build the API once, then run the suite with the contract dependencies installed:

```sh
hale build dna/api
python3 -m venv /tmp/hale-api-tests-venv
/tmp/hale-api-tests-venv/bin/python -m pip install -r dna/api/contract/v1/requirements.txt
/tmp/hale-api-tests-venv/bin/python -m unittest discover -s dna/api/tests -v
```

Set `HALE_API_BIN=/absolute/path/to/api` to test a binary built elsewhere. The
suite deliberately does not rebuild the service; this avoids concurrent builds
and makes the tested binary explicit. Every success/error API response is checked
against the checked-in JSON Schema. Missing `jsonschema`, schema or service binary
fails the suite rather than silently skipping a gate.

The fixtures need Git and loopback sockets. Each test owns a temporary directory,
ephemeral ports, service process group and optional Python identity provider.
Cleanup terminates only those processes, with bounded waits. Database/store URL
and evidence-key environment settings are removed from child environments. No
Postgres, Docker, organization body, external identity provider or model runs.

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

The identity-provider fixture follows the existing native OIDC test's direct
token-endpoint trust model. It does not test a production provider, TLS or token
signature validation. These tests do not replace the native principal/domain
tests, storage tests, browser interaction tests or complete service-stack gates.
