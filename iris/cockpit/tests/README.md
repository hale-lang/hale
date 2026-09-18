# Cockpit browser integration

These Playwright tests run Chromium against the real Hale API and temporary Git
Records. A native Hale writer imports the HTTP suite's fixtures, producing real
canonical practice receipts and journal events. No database, organization body,
model, external identity provider, or production Record is used.

From the repository root:

```sh
target/release/hale build dna/api
cd iris/cockpit
npm ci
npx playwright install chromium --only-shell
HALE_BIN="$PWD/../../target/release/hale" \
  HALE_API_BIN="$PWD/../../dna/api/api" npm test
```

The runner compiles its native fixture in owned temporary storage; it does not
rebuild the API. Both binary paths can point elsewhere. Git configuration and
plumbing overrides, database URLs and model credentials are isolated. Each test
owns its Record, dynamic listener and API child; failure teardown stops the child
and removes scratch storage. Tests have no automatic retries.

Practice/review navigation, opaque IDs, Unicode and multiline text, browser
history, missing objects, empty catalogs, snapshot conflicts and receipt
redaction use actual API responses. Authentication loss and source failure use
intercepted 401/503 responses to exercise browser state transitions; real OIDC
callbacks and authenticated reads remain covered by `dna/api/tests`. Delaying a
real detail response tests that obsolete data cannot overwrite another workspace.

Failure traces, screenshots and the API log land in `test-results`. Successful
desktop and narrow practice screenshots are also saved there for visual review.
CI runs the suite on partition 1 after the compiler and API have been built.
