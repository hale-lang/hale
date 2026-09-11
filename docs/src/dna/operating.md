# Operating the fleet

The organization can run on your workstation while the code it
governs runs on a fleet. The fleet is described by the plan Hale
already checks; each machine runs a node that expresses what the
record says; a deploy is a row in the record; and an approval's
window is over every instance the change touched.

## The plan

```json
{"schema": "1.2", "name": "production", "instances": [
  {"id": "api-0", "seed": "services/api", "node": "edge-1", "labels": ["api"]},
  {"id": "api-1", "seed": "services/api", "node": "edge-2", "labels": ["api"]},
  {"id": "worker-0", "seed": "services/worker", "node": "edge-1", "labels": ["worker"]}]}
```

An instance names the **seed** it is built from and the **node** that
runs it. Two instances of one seed on two nodes are two replicas.
Routes, groups and claims are the fleet's own — see [Multiple
binaries](../services/multi-binary.md) — and `hale fleet check`
composes the plan from artifacts cut from the seeds, so a change to
one service that breaks a claim over all of them is caught before
anyone reviews it.

```toml
[fleets]
production = "ops/production.plan.json"

[dna]
fleet = "production"          # the arrangement the DNA expresses
```

Services that are not Hale — a database, a queue — are environment:
they appear in the plan as endpoints the Hale services bind to, and
the DNA never edits them.

## The nodes

On each machine, a clone of the repository and one node:

```sh
git clone git@example.com:you/chat.git && cd chat
hale node edge-1                          # --repo <clone> --fleet <name> --tick <ms>
```

```text
hale node edge-1: expressing from /srv/chat (record via origin)
hale node edge-1: instance api-0 up (pid 4123) at 232dc8f18dde as 3c9b9327e480
hale node edge-1: fleet.deploy #41 expressed (2 instance(s) started)
```

A node syncs the record every tick, reads the latest deploy, and
when that names a revision it does not express yet, fetches it,
checks it out, builds the instances the plan assigns to it that the
deploy touches, restarts them, and reports each one into the record
in its own name: `instance.up` with the revision, the model hash and
the build digest; `instance.exited` with the code when one goes.
That is all it does. It never decides.

## Deploy, and what the record shows

```text
$ hale dna deploy HEAD
fleet.deploy #41: production at 232dc8f18dde touching api-0 api-1 worker-0

$ hale dna fleet
fleet production (ops/production.plan.json)
last deploy: deploy by operator 232dc8f18dde (232dc8f18dde) touching 3 — the whole genome
  api-0        edge-1     up       rev 232dc8f18dde model 3c9b9327e480
  api-1        edge-2     up       rev 232dc8f18dde model 3c9b9327e480
  worker-0     edge-1     up       rev 232dc8f18dde model 91ac4e0b7d21
```

`hale dna fleet` is read from the record, from any clone: every
instance of the plan, its node, whether it is up, the revision and
model hash it last came up at, and the deploy that asked. Two
replicas reporting two different hashes is something you can see.

## An approval, on the fleet

Under `hale dna run` with `[dna] fleet` set, an approval does what
`dev` does on one machine, over the fleet:

```text
hale dna run: m1 requests a restart (apply bf94e503c1c0… seed services/api fitness …); deploying to the fleet `production`
hale dna run: m1: fleet.deploy apply bf94e503c1c0 touching api-0 api-1
hale dna run: m1: 2 instance(s) observed healthy for 15s as 8517c3db7499
```

The host writes a deploy row naming the candidate and the instances
whose seed the change edited — `api-0` and `api-1`, not the worker —
and waits: every touched instance must come up at the revision, and
then none may exit for the observation window. All up and quiet:
`healthy`, and the change is retained. One falls over:

```text
hale dna run: m1: instance api-1 on edge-2 exited (137) inside the observation window
hale dna run: m1: fleet.deploy rollback to 232dc8f18dde touching api-0 api-1
```

`expression.crashed` names the instance and its node, the
organization rolls the genome back to the base, and the rollback is
another deploy row every node answers. Both replicas come back at
the base. `hale dna history m1` has the instance's name in it.

```sh
hale dna rollback m1        # the same rollback row, by hand, later
```

## Your own pipeline instead

If the fleet already has a deployer — a script, Nomad, Kubernetes —
wire it in as the deployment gateway in `dna/org/main.hl`:

```hale,fragment
deployment: dna::ShellDeployment { command: "ops/deploy.sh", seed: "services/api" },
```

The command is called as `ops/deploy.sh express <candidate> <seed>`
on an approval and `ops/deploy.sh rollback <base> <seed>` after a
bad window or a rejection. It owns expressing *and* judging: exit 0
from `express` means up and healthy, anything else is the
observation that rolls back. No host is asked; the organization
expresses through the command in its own handler and the record
says `expression.deployed`.

## What is not here yet

A Review's semantic diff is the edited seed's; the deploy row names
the instances it reaches, but there is no fleet-level structural
diff. Pressure from services' typed metrics has a spelling
(`hale dna pressure raise`) and nothing raising it for a node yet.
And the organization is not itself an instance of its plan.
