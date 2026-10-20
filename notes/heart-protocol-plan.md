# A heart in any language — plan

The organism's seam around the heart has two halves. The inbound half (events on `<org>.app.<app>.<event>`, one JSON object with an `id`, landed as `reading.recorded` through the organization's `heart` durable) is NATS and JSON and needs nothing of Hale. The outbound half ("the organism acts on the heart through its API, never its events") is today stated as a Hale surface and is the part #987 left unbuilt. This plan states the seam as a contract any codebase can implement, and proves it with a heart that is not Hale.

## 0. Attaching an existing app without changing it (H0)

Most hearts will be existing services that emit metrics and logs and nothing else. The organism attaches through what they emit and acts through what already controls them; the protocol below is for apps that choose to speak it.

- **The boundary.** An existing system is the heart, whole: the service and its monitoring, its alerting and its deploy pipeline. It stays the heart's until the organism builds a sense that replaces a piece of it, and each replacement is a reviewed operation, never a side effect of attaching. The organism starts by listening to what the heart already says about itself and acting through what already runs it; it owns nothing of the heart's on day one.

- **The sense adapter.** `hale dna application attach <name> --foreign --metrics URL --logs URL --health URL --deploy "cmd"` records the adapter's spec in `application.attached`, and the node runs one adapter process per attached app beside its other processes (stopped at `remove`). Its first stage is the heart's own Alertmanager webhook: the adapter receives the alerts the heart already raises on a route of its own, and each alert is a reading `app.<app>.<event>` with the alert fingerprint as its `id`, published on the organism's own account. The heart's alerting decides what is worth a reading; the organism adds no rule of its own yet. The second stage is the organism's own senses: PromQL over HTTP and LogQL against Loki (later OTLP), each a rule of the genome landing a reading with an `id` the adapter derives (the metric, window and threshold; the log stream and timestamp). The pulse is the metrics' `up` or the health route. From the organism's side the app is a heart: readings are signals, the reflexes and concerns read them unchanged.
- **Migrating a rule is one Review.** Moving a rule from the heart's alerting into a sense the genome owns is one Review: the genome's rule is ratified and the heart's alert it replaces is retired in the same operation, so a condition is never watched twice or not at all. What has not been migrated stays the heart's.
- **No expression gateway for a foreign heart.** The organism changes nothing of a foreign heart's running state: a restart or a deploy is a Board action carried out through the heart's own pipeline, and the observation window judges the pulse it already exposes. A habitat-scoped heart keeps the gateway (`systemctl`, `docker compose`, the habitat's hands, #1477).
- **First targets.** Two small internal services of a Go monorepo from a downstream handoff: one proves the health path (the pulse from a health route, a restart judged by it), the other the metrics path (an alert from the heart's own alerting landing as a reading).
- **Reference.** A Go service under `dna/tests/heart-go/` that serves `/metrics` and `/healthz` and logs to stdout (collected by a Loki in the DNA compose) and nothing else; the DNA suite attaches it, a threshold rule lands a reading, and a leg reads it through its imported surface (H1). Zero lines change in the service.
- **The bootstrap is a recipe the tooling enforces.** One manifest in the organism's genome (`dna/applications/<name>.toml` or the record's `application.attached` body) says what the app emits (metrics URL, log source, health route), what runs it (systemd, compose, kube, a pipeline), how it deploys (a command or a dispatch) and the thresholds that become readings; the app's repository changes by zero lines and its language never appears. `attach --check` and `hale dna doctor` verify every wire before anything runs: the scrape reachable, the health route answering, the log source yielding, the deploy command dry-running, the broker account drawn, one synthetic reading landed end to end; a bootstrap that passes is done, one that does not names the missing wire. Repeatability is measured: someone who has not seen the code bootstraps the reference service from the chapter and the manifest alone in under an hour with the check green; a second reference service in another language later proves the manifest is all that varies.
- **What the stdlib already has:** the HTTP client and server, JSON, timers. New is the adapter program, the attach verb's `--foreign` spec and rows, the rule vocabulary for metrics and logs in the genome.

## 1. The outbound contract is declared surfaces (H1)

A heart's outbound contract is one or more api surfaces the organism declares and consumes, as the standard says of anything that crosses a boundary (docs/src/dna/standard.md, "anything that crosses a boundary is declared").

- **Whose surfaces.** A Hale heart's surfaces are its own. A foreign heart's are generated from its MCP tool list or its OpenAPI document by `hale api import`, committed to the organism's genome, and drift-checked at attach: a heart whose tools or operations no longer match the committed surface is a visible change, refused until a Review records the new one.
- **Bound by a client param of main.** Each surface is bound by a client param, `api::client(Surface, mcp::Client | http::Client)`, and its rows are typed methods. A distributed heart declares one surface per endpoint.
- **A leg's tools are rows.** A leg's tools are the rows its roles admit. Every call is an effect row and a receipt naming the surface and the row, so a leg's conclusion names the evidence it read.
- **Auth.** The credential an endpoint needs lives in the vault under the application's name; the organism hands it to the client, never to a leg's prompt.
- **Inbound is unchanged.** Inbound stays topic bindings.

## 2. Conformance (H2): struck

There is no `Heart` surface a heart must serve and no conformance verb pointed at it. Conformance is the import's drift check (H1).

## 3. Expression (H3)

Deploys of a foreign heart are Board actions carried out through the heart's own pipeline; the organism holds no credential that changes its running state, and no gateway applies anything to it. A habitat-scoped heart deploys through #1477.

## 4. How many: codebases and applications

The seam is plural on both axes, and the two axes are different things. A **codebase** is a source the organism knows how to build or run: a Hale workspace, a repository of compose services, a foreign tree with an adapter. An **application** is what the record attaches by name: an account on the nerves (`app-<name>`, GH #989), readings keyed `<app>/<event>/<id>`, an expression (a topology and a deploy target, or none for a foreign one). One codebase can yield several applications (a workspace with several mains), and one organism can attach applications from several codebases. Nothing in the inbound half assumes one: the heart durable reads `<org>.app.>`, accounts and readings are already per name. What assumes one is the expression half, and that is what this item changes.

- **A codebase comes first.** A codebase may be attached with no application: its path, its build command, its test command and the hands a leg needs there. A leg's worktree is a branch of it, and its PRs open there. Readings and the application row come later, if the codebase is also a heart.
- **`application.attached` is one row per application**, and the record holds as many as the organism attaches: `name` (the subject token), `source` (the codebase and the main or service within it), `scope: habitat | foreign`, the expression fields (`artifact`, `artifact_digest`, `shape_hash`; empty for an adapter-attached app), `provenance`. `application.detached` revokes one. `hale dna init` attaches what the workspace declares (one main, or every `apps/*` main it is told to); `hale dna application attach <name> [--main PATH | --foreign ...]` adds one later; `remove` is already there.
- **Expression is per application.** `current.topology` becomes `current/<name>.topology`, the upgrade and deploy paths take the application's name, the fleet and effects gates run per application, and `ShellDeployment` is one per application. The host's projection lists the applications (the `expression` object becomes an array keyed by name); the face's Application picker, which already exists, selects among them.
- **Scope.** A `habitat`-scoped heart is one the habitat builds and runs (voice is the worked example: its image, its deploy and its senses are the habitat's to change); a `foreign`-scoped heart owns its infrastructure, and the organism reaches it only through the attach boundary: its alerts, metrics and logs inbound, its own deploy path outbound. Anything that reaches past a foreign heart's boundary (its cluster, its pipeline's configuration, its monitoring's rules) is refused by rule, not by convention.
- **What stays singular:** the record and its journal, the organization and its bus token `<org>`, authority and roles, the practices, and the organism's place in the habitat. Several applications are one organism's; several records are the habitat's.
- **Identity:** an application's name is unique within the organism and is the only key the nerves, the readings, the expression files and the heart's surfaces (H1) use; a codebase has no account of its own.

This lands before H0's adapter attach (which is the first second application most organisms will have) as the model change under it: the plural row, the per-name expression paths and the projection's list, with `init` and the existing single-application fixtures unchanged in behaviour (one row, one topology file under the new name).

## Exit criteria

First slice: one human intent, one leg PR on the foreign codebase with receipts, merged by the team.

H0: the recipe above, measured as stated, on the Go reference service. The reference foreign heart attaches and its readings land; a leg calls its imported surface with a receipt per call; a Hale heart still passes unchanged.

## Order and size

Item 4 first, the codebase axis before the application rows (the model change the rest stands on: the codebase attach, then the plural row, the per-name expression paths and the projection), then H1 (the declared surfaces and their client binding, which is what a leg needs to work on a foreign codebase), then H0's senses. One PR each.
