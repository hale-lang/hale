# Initial Hale API read contract

This experimental contract describes the implemented read-only DNA slice of
`/api/hale/v1`. It is a subset of the cockpit proposal, not completion of the
service-development plan. `openapi.json` lists the routes; `schema.json` supplies
JSON Schema 2020-12 response definitions shared by fixtures and live HTTP tests.

The application id is the native Record genesis identity. Collection item ids
are opaque, including any slashes; use URL encoding for query values. A request
with `id` returns a one-item collection, or 404 when absent. Unknown or repeated
query fields are rejected. Acting-position context, commands, remote CLI credentials,
generic runtime observation and recursive execution are not advertised yet.

Every successful response names the local Record identity, head and decimal
revision. This is the inspected local snapshot, not a claim that a remote clone
has synchronized or a Ledger projection is current. Pagination after the first
page requires its snapshot; a changed head returns 409 and the client restarts
the read. No time or global cross-store ordering is inferred from a revision.

`/dna/organization` additionally names its checked source commit, dependency
digest/origin, compiler artifact and static coverage in `data.basis`. Its opaque
snapshot binds those inputs and the Record head. Nodes use exact compiler
instance paths; only an explicit `positions` declaration group marks a role as
`position`. Ownership maps are returned separately without inventing an identity
join or grants. Subscription capacity and supervision retry are decimal strings
or null, preserving values outside JavaScript's exact integer range.

Practices distinguish proposal, ratification, retirement and decline. A Review
can be settled while its practice is still unratified. `settled` and
`review_settled` preserve native display text. Use `outcome` / `review_outcome`
for a recognized native Review decision, separately from practice activation.
`author` is the knowledge locus (often `org`); `requester` and `rationale`
attribute the proposal to the person and stated reason. They do not identify
the currently authenticated reader or confer permission to act.

Unavailable text is empty with an explicit availability/status field. The
first slice does not claim complete receipt visibility after Ledger adoption;
it withholds content whose current restrictions cannot be checked from Record,
including after Ledger abandonment, which does not restore those restrictions.
Derived Review questions are subject to the same restriction. It never reads
the private protected-body store or returns raw review reasoning/diff blobs.
For an ordinary non-knowledge Review, `text_status=not_applicable` indicates
that no practice receipt governs the question declared directly in Record;
`text_available=false` then refers to the absent practice receipt, not the
visibility of that declared question. Known/unknown subject restrictions still
suppress it through the other status values.

Local mode trusts access to the loopback listener. OIDC mode uses the configured
issuer and mapped-member session and fails closed when authentication is absent.
The OpenAPI security alternatives describe these deployment modes; a caller
cannot switch an OIDC service to local mode through a request. This is not a
position-scoped authorization API or a deployment-ready public ingress.

Run the native Hale contract tests from the repository root:

```sh
HALE_API_CONTRACT_ROOT="$PWD/dna/api/contract/v1" \
  hale test dna/api/contract/v1/tests
```

The tests preserve the original nine fixtures plus organization fixtures, reject
unexplained actor fields, check all five read routes and their response-schema links, and exercise the
validator's rejection paths. To check a captured response:

```sh
hale build dna/api/contract/v1/check
HALE_API_CONTRACT_ROOT="$PWD/dna/api/contract/v1" \
  dna/api/contract/v1/check/check PracticesResponse result.json
```

Running `check/check` without arguments checks the files and fixtures alone.
The live HTTP suite imports `validator` and uses a let-bound instance:

```hale
import "../contract/v1/validator" as wire_contract;

let validator = wire_contract::Validator { root: contract_root };
let result = validator.validate(schema_name, response_json);
```

The result is `ValidationResult { ok, error }`. Missing or malformed schema files
fail the check. No Python packages, other language runtime, network access or
schema downloads are needed.

The native validator implements a **bounded contract profile**, not all of
JSON Schema 2020-12 or OpenAPI. It supports the keywords used here: object,
array, string, integer, boolean and null `type`; `properties`, `required`,
`additionalProperties: false`, `items`; local `#/$defs/Name` references;
string/boolean `const`, unique string `enum`; `minLength` from 0 to 1,000,000;
integer `minimum`/`maximum` paired with `type: integer`; the exact unsigned
decimal-string `pattern`; `allOf`, nonempty `anyOf`, and paired `if`/`then`. `$schema`,
`$defs`, `title` and `description` are recognized. Unknown keywords,
unsupported keyword values, unresolved or cyclic references fail before
response validation. Future schema additions therefore need explicit validator
support and tests.

Wire integers use JSON integer tokens (`1`, not `1.0` or `1e0`); comparisons
do not truncate to machine integers. Object keys must be literal, unique and
unescaped, and cannot contain `|`. JSON nesting and schema/evaluation depth
are bounded at 64. String values support UTF-8 and paired Unicode escapes;
`minLength` counts decoded Unicode scalar values. Invalid UTF-8, unpaired
surrogates and escaped U+0000 are rejected; the latter avoids native
NUL-terminated string truncation during constant and enum comparisons.
These narrower wire/profile rules are intentional and do not claim general
JSON Schema conformance. OpenAPI checks cover this release's routes and local
response references, not full specification validation.
