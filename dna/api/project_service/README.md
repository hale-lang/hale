# The cockpit head: project service

`dna/api/project_service` is the one process the Iris cockpit talks to
(GH #965). It serves the browser shell, owns the operator-machine state
under a state directory — a project registry, a receipt journal, one
directory per run, the pid files of its children — and reverse-proxies
every `/api/hale/v1/applications…` request to the attached project's
API child (`dna/api/practice_review <root> <api-port>`) on the loopback.
Every operation is the CLI verb (`$HALE_BIN dna …`) run **detached**
under `timeout -k 10 <secs> sh -e`; the head re-implements nothing and
appends nothing to any Record itself.

```sh
hale build dna/api/practice_review
hale build dna/api/project_service
HALE_BIN="$(command -v hale)" \
  ./dna/api/project_service/project_service 8792 iris/cockpit/web \
  dna/api/practice_review/practice_review 8793 [/absolute/path/project]
```

`iris/cockpit/start.sh [PROJECT]` does the build and the launch. The
routes, envelopes and operations are described in
[`dna/api/README.md`](../README.md#head-project-service) and pinned by
[`dna/api/contract/v1`](../contract/v1/README.md#head).

## Files

| file | holds |
|---|---|
| `main.hl` | argv and exit codes, the state directory, `head.pid` / `head.url`, child re-adoption, the restore of the last activated project, the startup attach, the server |
| `head.hl` | the handler: the four head routes, the proxy, settle-on-request, the API child's lifecycle, attach and detach |
| `operations.hl` | S2's nine operations, the request-shape check, the row-writing list, the `record` evidence, the catalog credential scan |
| `operations_s3.hl` | the sixteen body, secret, model, connection, handoff and observer operations (lane S3's file) |
| `journal.hl` | `HeadJournal` over `receipts.jsonl`: identity, fingerprint, replay, settle |
| `registry.hl` | `Registry` over `projects.jsonl`: register, activate, deactivate, forget, restore |
| `policy.hl` | the two per-project policies the head synthesizes when absent, and their decode checks |
| `proxy.hl` | the one-connection HTTP exchange: the proxy, the readiness probe |
| `children.hl` | detached children with pid/exit/log files (the host's `procs.hl` idiom), `.cmd` digests for re-adoption, log pages, private files |
| `types.hl` | `HeadContext`, `Plan`, `ChildView`, `Receipt`, `Registered`, the `Outcomes` interface |

## State

```text
STATE = ${HALE_IRIS_HEAD_STATE:-${XDG_STATE_HOME:-$HOME/.local/state}/hale/iris/head}
STATE/head.pid  head.url                     the head holding this state, and where it listens
STATE/projects.jsonl  receipts.jsonl         the registry and the receipt journal, append-only
STATE/runs/<command_id>/{script.sh,run.pid,run.exit,run.log,run.cmd}
STATE/children/<api|body|observer>.{pid,exit,log,cmd}
<root>/.hale/dna/iris/{authority.json,task-policy.json}   synthesized only when absent
${XDG_CONFIG_HOME:-$HOME/.config}/hale-dna/sources/<NAME>  operator-written secret sources (0600, one line)
```

## Tests

```sh
export HALE_BIN=/abs/hale HALE_API_CONTRACT_ROOT=$PWD/dna/api/contract/v1
HALE_HEAD_BIN=$PWD/dna/api/project_service/project_service HALE_API_BIN=$PWD/dna/api/api \
  hale test dna/api/project_service/tests
```

`journal_test.hl` runs the journal and the registry over a fake runner;
`operations_test.hl` the plans, refusals, attach and detach without
HTTP; `head_api_test.hl` the head over HTTP with a restart. Each makes
its own scratch root and takes its ports from `dna::free_port`; a
failing fixture is built with `hale build <file>` and run by hand to
see its stderr.
