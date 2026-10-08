# A heart in any language — plan

The organism's seam around the heart has two halves. The inbound half (events on `<org>.app.<app>.<event>`, one JSON object with an `id`, landed as `reading.recorded` through the organization's `heart` durable) is NATS and JSON and needs nothing of Hale. The outbound half ("the organism acts on the heart through its API, never its events") is today stated as a Hale surface and is the part #987 left unbuilt. This plan states the seam as a contract any codebase can implement, and proves it with a heart that is not Hale.

## 0. Attaching an existing app without changing it (H0)

Most hearts will be existing services that emit metrics and logs and nothing else. The organism attaches through what they emit and acts through what already controls them; the protocol below is for apps that choose to speak it.

- **The sense adapter.** `hale dna application attach <name> --foreign --metrics URL --logs URL --health URL --deploy "cmd"` records the adapter's spec in `application.attached`, and the node runs one adapter process per attached app beside its other processes (stopped at `remove`). The adapter queries Prometheus (PromQL over HTTP; Alertmanager webhooks received on a route of its own), tails Loki (LogQL), and later takes OTLP; each signal a rule of the genome names lands as a reading `app.<app>.<event>` with an `id` the adapter derives (the alert fingerprint; the log stream and timestamp; the metric, window and threshold), published on the organism's own account. The pulse is the metrics' `up` or the health route. From the organism's side the app is a heart: readings are signals, the reflexes and concerns read them unchanged.
- **The expression gateway.** The `HeartHand` and `DeployHand` act through what runs the app: the shell gateway (`systemctl`, `docker compose`, `kubectl`), the app's own pipeline through a workflow dispatch, a restart through the orchestrator; the observation window judges the same pulse.
- **Reference.** A Go service under `dna/tests/heart-go/` that serves `/metrics` and `/healthz` and logs to stdout (collected by a Loki in the DNA compose) and nothing else; the DNA suite attaches it, a threshold rule lands a reading, a reflex restarts it through the gateway, and the window judges it. Zero lines change in the service.
- **The bootstrap is a recipe the tooling enforces.** One manifest in the organism's genome (`dna/applications/<name>.toml` or the record's `application.attached` body) says what the app emits (metrics URL, log source, health route), what runs it (systemd, compose, kube, a pipeline), how it deploys (a command or a dispatch) and the thresholds that become readings; the app's repository changes by zero lines and its language never appears. `attach --check` and `hale dna doctor` verify every wire before anything runs: the scrape reachable, the health route answering, the log source yielding, the deploy command dry-running, the broker account drawn, one synthetic reading landed end to end; a bootstrap that passes is done, one that does not names the missing wire. Repeatability is measured: someone who has not seen the code bootstraps the reference service from the chapter and the manifest alone in under an hour with the check green; a second reference service in another language later proves the manifest is all that varies.
- **What the stdlib already has:** the HTTP client and server, JSON, timers. New is the adapter program, the attach verb's `--foreign` spec and rows, the rule vocabulary for metrics and logs in the genome.

## 1. The heart protocol is a contract the organism owns (H1)

- `dna/core/heart/surface.hl` declares `api Heart { … }`: the calls the organism makes on a heart. The first set is what #987's `HeartHand` and `DeployHand` need: `pulse` (is it alive, since when, which expression), `describe` (what it is: name, version, the genome revision it expresses), `apply` or `deploy` hooks where the heart, not the body, expresses a change, and `observe` (the heart's own health during the observation window). Each row carries `requires` by organism role (operator, the node, the spine).
- The inbound side is declared as the stream contract beside it: the envelope (`id`, the subject rule, the size and id bounds the host already enforces in `pulse.hl`), so a foreign heart has one document for both directions.
- `hale api export --surface Heart` produces the bundle a foreign codebase builds against: the description, OpenAPI, JSON Schema, `.proto`, MCP, the digest, and recorded exchanges for every outcome, committed under `dna/core/heart/contract/`; the drift check guards it.
- Auth: the organism is the caller. At `application.attached` the organism draws a per-app bearer the heart verifies (handed over as `HALE_DNA_HEART_TOKEN` beside the four NATS variables; rotated at `upgrade`; revoked at `remove`). A foreign heart checks a bearer equality; a Hale heart uses a `StaticRoles`-style source. The organism holds the token in the vault like the NATS credential.
- A spec chapter, `docs/src/dna/heart-foreign.md` and `spec/dna.md` § A heart in any language: the wire end to end, in the order a foreign implementer needs it: attach, credentials, publish an event, serve the surface, the five outcomes, remove.

## 2. Conformance (H2)

- `hale api conform --surface Heart --endpoint URL [--token T]` replays the contract's recorded exchanges against a live implementation and prints a report (each outcome: pass, or the first byte that differs); it is the same replay the generated-client test uses, pointed outward.
- A reference foreign heart under `dna/tests/heart-foreign/` (Node, since the face lanes already have node in CI; about a hundred lines): publishes events that land as readings, serves the `Heart` surface over HTTP from the exported OpenAPI, verifies the bearer. The DNA suite runs it: readings land, `conform` passes, and the `HeartHand` reaches it through the generated Hale client, so the hand never knows the heart's language.
- `hale dna application attach` grows a `--foreign` note in its output: the variables and the bundle path a foreign heart needs.

## 3. Expression of a foreign codebase (H3)

- The shell deployment gateway (`dna::ShellDeployment`) is the documented and tested path: apply runs the gateway (a build or a container restart the project owns), the observation window judges the pulse, which is language-neutral; `heart.md` § Expression gains the foreign case with the reference heart.
- Out of scope: a foreign codebase as the organism itself; a heart that cannot speak NATS (an HTTP ingestion bridge is an open point).

## Exit criteria

H0: the recipe above, measured as stated, on the Go reference service. The reference foreign heart attaches, publishes, serves, conforms and is driven by the `HeartHand` in the DNA suite; the contract bundle is committed and drift-checked; the spec chapter is the only document a foreign implementer needs; a Hale heart still passes unchanged.

## Order and size

H0 first (it is what most hearts are), then H1, H2, H3; one PR each; H1 is mostly declaration and documentation (one pane), H2 adds the verb and the reference heart (two panes: the verb, the heart and its DNA test), H3 is documentation plus one DNA test. The `HeartHand`'s own calls (#987) land with H2.
