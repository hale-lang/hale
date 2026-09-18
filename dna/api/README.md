# DNA read API

The first service API slice reads practices and reviews from an existing local
DNA Record. It runs independently of the organization body, Postgres and Iris.
Typed projections in `dna/operations` are shared with the native CLI. The API
does not execute CLI commands or parse terminal output.

This is an experimental source-built service. There is no `hale dna api` launcher,
Compose service profile or browser cockpit in this slice. The remaining work is
tracked in [SERVICE-DEVELOPMENT-PLAN.md](../SERVICE-DEVELOPMENT-PLAN.md).

## Run

From the Hale source checkout, using a current Hale compiler:

```sh
hale build dna/api
./dna/api/api /absolute/path/to/a/dna-project 8792
```

The project must already have a DNA Record (`hale dna new` / `init`). The server
binds only `127.0.0.1`. Stop it with Ctrl-C. Discover the native application id:

```sh
curl http://127.0.0.1:8792/api/hale/v1/applications
```

Use the returned id in the following routes:

| GET route | Result |
| --- | --- |
| `/api/hale/v1/applications/{id}/capabilities` | Principal mode, implemented reads and unavailable mutations |
| `/api/hale/v1/applications/{id}/dna/practices` | Named practice proposals, lifecycle, attribution and available canonical text |
| `/api/hale/v1/applications/{id}/dna/reviews` | Review identity, exact subject, required authority and recorded decision |

Collections accept `id`, `limit`, `offset` and `snapshot`. Pass opaque ids through
URL query encoding, for example `curl --get --data-urlencode 'id=org/reviews/one'`.
The default page is 25 items, maximum 100. Later pages require the preceding
Record-head snapshot; 409 means restart pagination. Errors have structured JSON
codes and HTTP status. A missing/corrupt Record is unavailable, not an empty
successful catalog. This first implementation bounds snapshots to 10,000 rows
and 16 MiB; exceeding the bound returns an explicit 503.

Each response states its local Record identity, head and revision. Reads do not
fetch a remote or claim remote freshness. A settled approval does not imply
practice activation. An unprocessed practice request is not listed as a practice
until the organization creates its actual document/proposal.

## Identity and content

With no configured principal source, or `dna.principal=local`, loopback access is
trusted and the reader is server-derived. `dna.principal=oidc` reuses the existing
issuer, client, redirect and member mapping configuration. Start at `/auth/login`;
`dna.oidc.redirect` must point to this service's `/auth/callback`. The service uses
`HALE_DNA_OIDC_SECRET` for the configured exchange. Missing, unmapped or expired
sessions cannot read the API. Unknown principal modes fail startup. No query
parameter changes the authenticated member or supplies an acting role.

Sessions are in memory. Restart requires another login; Record identities and
read results persist. The existing UI's legacy command routes are never routed
through this server. A mapped member has the existing Record-reading scope;
position-scoped permissions and remote CLI tokens are future work.

Practice documents must match their content digest, proposal metadata and a
supported native canonical encoding. Redacted/protected text is suppressed even
when a stale local blob remains. After any Ledger adoption the API still serves
Record metadata, but withholds canonical text and derived Review questions until
a later adapter can establish current Ledger receipt visibility. Typed decision
outcomes remain distinct from withheld display text. The legacy local CLI keeps
its existing receipt-rendering behavior.
Ledger abandonment does not restore visibility: it does not copy historical
receipt restrictions back into Record. Unknown classification metadata and
conflicting Review subject references likewise cannot establish readable text.

See the [contract](contract/v1/README.md) for response schemas and the
[query layer](../operations/README.md) for projection semantics.

## Validate

From the source checkout, run the native Hale contract and integration tests:

```sh
export HALE_API_CONTRACT_ROOT="$PWD/dna/api/contract/v1"
hale test dna/api/contract/v1/tests
hale check dna/api
hale build dna/api
HALE_API_BIN="$PWD/dna/api/api" hale test dna/api/tests
hale test dna/operations/tests
cargo test -p hale-dna -p hale-iris
```

The integration suite starts only owned loopback processes and temporary Git
repositories. It validates live responses against the same contract as the
fixtures, including a local OIDC issuer, restart, pagination, refusal
and content suppression. It requires socket access; no paid model, production
database or organization body is involved. `HALE_API_BIN` selects an already
built service. CI builds with the checkout's compiler and runs this suite.
The native contract checker supports the schema profile used by this API and
rejects unsupported schema constructs; it is not a general JSON Schema validator.
