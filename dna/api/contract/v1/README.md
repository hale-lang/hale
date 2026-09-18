# Initial Hale API read contract

This experimental contract describes the implemented read-only DNA slice of
`/api/hale/v1`. It is a subset of the cockpit proposal, not completion of the
service-development plan. `openapi.json` lists the routes; `schema.json` supplies
JSON Schema 2020-12 response definitions shared by fixtures and live HTTP tests.

The application id is the native Record genesis identity. Collection item ids
are opaque, including any slashes; use URL encoding for query values. A request
with `id` returns a one-item collection, or 404 when absent. Unknown or repeated
query fields are rejected. Position selection, commands, remote CLI credentials,
generic runtime observation and recursive execution are not advertised yet.

Every successful response names the local Record identity, head and decimal
revision. This is the inspected local snapshot, not a claim that a remote clone
has synchronized or a Ledger projection is current. Pagination after the first
page requires its snapshot; a changed head returns 409 and the client restarts
the read. No time or global cross-store ordering is inferred from a revision.

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

Install `requirements.txt` into a virtual environment, then run:

```sh
python dna/api/contract/v1/validate.py
```

To check a captured response, add `--schema PracticesResponse --response result.json`.
The live integration suite uses these same validators; schema validation is a
required dependency, not a silently skipped check. There is no schema download
at test time. The validator checks schema/example conformance and the OpenAPI
references; it is not a full OpenAPI-specification validator.
