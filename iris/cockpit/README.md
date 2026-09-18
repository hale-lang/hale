# Iris cockpit

A browser surface for inspecting a Hale application's declared organization,
DNA practices and reviews.
The frontend is three static files, served beside the native Hale API. It has no
build step, runtime package dependencies, database connection or domain engine.

## Run locally

From a Hale source checkout, with a current Hale compiler and an existing DNA
project:

```sh
hale build dna/api
./dna/api/api /absolute/path/to/dna-project 8792 "$PWD/iris/cockpit/web"
```

Open <http://127.0.0.1:8792/>. The API binds to loopback. Omitting the final
webroot argument preserves the API-only service. The webroot is this static asset
directory, not the DNA project or its Record. Only the three named assets are
served; the service is not a general file server.

The project chooses trusted local access or its existing OIDC configuration;
see [API identity and content](../../dna/api/README.md#identity-and-content).
The shell and assets contain no Record data and can load before sign-in. Every
data request still passes through the API's authentication boundary. The OIDC
callback must point to this service's `/auth/callback` on the same origin.

This remains an experimental source-built entry point. It does not add a
`hale dna api` launcher, a complete Compose profile or a hosted deployment.

## Current surface

- **Practices:** paged proposals and revisions, available document text,
  lifecycle, provenance, rationale, governing Review and superseded digest.
- **Reviews:** exact subject, required authority and recorded decision, linked
  back to the practice when one is identified. Approval and activation remain
  separate facts.
- **Organization:** source-backed static instance outline and inspector,
  explicit position groups, typed parameters and message/supervision contracts,
  source files, dependency fingerprints and separate declared ownership maps.
  Selecting a node changes the inspection context while preserving the signed-in
  principal. Static declarations do not establish an occupant or effective grant.
- **Runtime:** an explicit connection link to the existing Iris observer. It
  remains usable without a DNA connection or session. It does not assert that
  the observer and Record refer to the same application, embed the collector,
  or proxy its events.
- **Knowledge and Definitions:** visible core workspaces whose read adapters
  are not yet available. Acting-position permissions and administrative actions
  require their authoritative service contracts.

Practice and Review URLs use browser hash routes and preserve opaque native IDs
through query encoding. Browser back/forward navigation works without requiring
a server route for every object.

The source footer identifies the inspected Record head and revision. Refresh
reads that local Record again; it does not fetch a remote. Paging holds the
original snapshot, and a 409 restarts the page sequence with a visible notice.
Missing objects, unavailable sources, empty catalogs and sign-in requirements
have distinct states. Old content is cleared when a request can no longer
establish readable data; records are not persisted in browser storage.

Receipt visibility remains the API's decision. After any Ledger adoption this
adapter exposes metadata but withholds receipt-derived text until an adapter
can prove current Ledger visibility, including after abandonment. The browser
explains the returned availability status; it cannot restore suppressed text.

## Development and verification

Edit `web/index.html`, `web/styles.css` and `web/app.js`, restart the API to load
the changed assets, then reload the browser.
There are no external scripts, fonts or asset services. JavaScript renders
native content as text and sends read requests only.

The native server tests cover HTTP, authentication and asset-serving boundaries:

```sh
export HALE_API_CONTRACT_ROOT="$PWD/dna/api/contract/v1"
hale build dna/api
HALE_BIN="$(command -v hale)" HALE_API_BIN="$PWD/dna/api/api" hale test dna/api/tests
```

Browser interaction tests live in `tests/`. They use a temporary Git Record
populated by native Hale writers and the real API; focused response overrides
exercise error and reconnect paths. Node/Playwright are test tooling only.

From the repository root, after building the API:

```sh
export HALE_BIN="$PWD/target/release/hale"
export HALE_API_BIN="$PWD/dna/api/api"
cd iris/cockpit
npm ci
npx playwright install chromium --only-shell
npm test
```

Set `HALE_BIN` to another current compiler if necessary. Linux environments may
need Playwright's browser system dependencies (`npx playwright install --with-deps
chromium --only-shell`); CI installs those and runs this suite in partition 1.
The browser tests exercise a real 404, receipt redaction and snapshot conflict;
controlled HTTP responses cover lost authentication and source unavailability.
The native suite separately exercises the real OIDC exchange and callback.

See the [readiness assessment](../COCKPIT-READINESS.md) and
[service development plan](../../dna/SERVICE-DEVELOPMENT-PLAN.md) for the first
administrative loop and the remaining core workspaces.
