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

Organization tests commit the native seed in `tests/organization` and a separate
domain ownership map into their temporary Git Record. The API invokes the actual
compiler against that committed source; tests do not substitute a hand-authored
topology artifact. Repeated nested instances, the explicit positions group,
structure outside that group and an uninstantiated declaration remain distinct.
Dirty source, changed committed source and invalid committed source exercise the
real provenance, conflict and failure paths. These are declaration inspection
tests, not proof of semantic position administration or live occupants.
One fixture uses ordinary `hale dna new` with an owned toolchain cache, preserving
the generated, ignored `vendor/dna` import. It never runs the organization or a
model. Valid, invalid and missing ignored dependency bytes exercise cache
invalidation at unchanged source HEAD, recovery and preservation of project refs,
worktree state and dependency contents. A larger native source fixture covers
parent inspection outside the current page and return to the original scope.

Failure traces, screenshots and the API log land in `test-results`. Successful
desktop and narrow practice screenshots are also saved there for visual review.
CI runs the suite on partition 1 after the compiler and API have been built.
