<!--
DRAFT. This page describes a path through the phase 4 work, and some of its
steps are in pull requests that are not on main yet. Each one is marked here;
remove its line when it lands.

- `hale habitat place`, `apply [<host>]`, `graft --host`, `model-map` and
  `model-probe`, and the face's infrastructure and organisms screens that
  drive them: D4 (dna/habitat-placement), D5 (dna/acceptance-homelab) and the
  face (face/operator-actions).
- `hale habitat brief` (the habitat's brief for an agent, which the last
  section mentions): D7 (#1728).
- The bundled nursery's `voice` entry must pin a revision of hale-lang/voice
  that builds from a fetched tree: voice's Dockerfile takes its toolchain from
  `.hale-toolchain/` of the build context (voice#45), which a fetched revision
  does not have, so the build step below runs `scripts/hale-toolchain.sh` in
  the fetched tree first. That step and the node's enrollment under the
  habitat's compose project (step 6) were not run end to end when this was
  written; the fixture that runs the rest of the path in CI
  (dna/tests/acceptance_homelab_test.hl) stands a small tree in for voice.
- The ten minutes are the time on a machine that has the voice image built
  already or pulls it; the first build of the toolchain is not counted.
-->

# Ask it something

From a machine with Docker Compose and the toolchain to a streamed answer from a model, through voice, in the habitat you just made. You need `hale` on your PATH, `docker compose`, `git`, and a browser. Nothing else is installed by hand: the habitat provisions its own database and message broker, and voice is fetched from the toolchain's nursery.

## 1. Start the habitat

```sh
hale habitat up
hale habitat status
```

`up` births the habitat the first time (no parent; its law and positions proposed to its Board), brings its memory, nerves and senses' store up under one compose project, records the host `local`, and starts its body as a systemd user unit (`--foreground` runs it in this terminal instead). It is idempotent: run it again whenever. `status` prints the genome, the body, each allocation, and the face:

```text
face    http://127.0.0.1:<port>/ (up, ...)
```

## 2. Sign in

Open that address. The head's gate sends the browser to the habitat's own identity provider, which on a local habitat signs you in as the subject `local-sub`, mapped to your user, and sends you back; the session lasts eight hours. There is no password to set. The face shows the habitat's name and state, its record revision and the organisms grafted so far (none yet), and the screens along its top: hardware, infrastructure, organisms ([The head and the face](./dna/head.md)).

## 3. Graft voice

voice is a sapling in the toolchain's nursery. On this machine:

```sh
hale habitat nursery list
hale habitat graft voice
```

`graft` fetches the pinned revision of voice into the habitat's store (`<habitat>/saplings/voice`), checks its digest, grafts it as the habitat's child, allocates the roles its `hale.toml` asks for (memory, nerves, a database and a broker), and records the sapling, revision and digest on the graft's row. A graft takes minutes and says what it is doing as it goes; the organisms screen follows the same graft from the **graft** button on voice's row, with a host picker.

To run voice on another machine of your LAN instead, add the machine first and graft with `--host`:

```sh
hale habitat host add rack-a.lan --user ops
```

This prints the machine's host key, which is trusted from then on, and the habitat's public key as the one line to append to `~/.ssh/authorized_keys` on `rack-a.lan`. Append it, then:

```sh
hale habitat host probe rack-a.lan
hale habitat graft voice --host rack-a.lan
```

`probe` finds the host `ready` and says what it offers (docker compose, its OS and architecture); `--host` places the new organism's memory, nerves, database, broker and its own services (the compose file at voice's root) on that host. The hardware screen shows the same: the host list, its offers and the fingerprint first seen, and the allocations on each.

## 4. Bring it up

`graft` places; `apply` runs:

```sh
hale habitat apply
```

`apply` brings each allocation up where it is placed, by that host's provider, and prints the pulse: every allocation with its host and what its provider observes of it (`up`, `degraded` or `down`). Run it again until voice's services are up; the infrastructure screen has an **apply** button that shows the same pulse. To move an allocation to another host first: `hale habitat place <key>/<role> <host>`, then `apply`. A memory that holds its organism's record moves only with `hale habitat apply --fresh`, which starts it empty on the new host.

voice's services are voice's own `compose.yaml`: Postgres, NATS, the api, the brain and a node. Its image is built from voice's tree with a toolchain, so make one there once, on the host that runs it:

```sh
cd ~/.local/share/hale/habitat/local/saplings/voice   # on the host that runs voice
scripts/hale-toolchain.sh
hale habitat apply
```

When it is up, voice's api answers on port 8080 of that host:

```sh
curl http://localhost:8080/healthz      # or http://rack-a.lan:8080/healthz
```

## 5. Set up voice's model

voice serves models that are configured on its admin plane: a model, the account that pays for it, a project with a key, and a node with a seat that reaches the model. The admin UI is at `/` of the same port; these are the same calls, for a seat running the static engine (it answers a fixed text, so no model needs to be installed):

```sh
api=localhost:8080; json='content-type: application/json'
curl -X POST $api/admin/v1/models   -H "$json" -d '{"id": "echo-1"}'
curl -X POST $api/admin/v1/accounts -H "$json" -d '{"id": "static", "name": "Static"}'
curl -X POST $api/admin/v1/projects -H "$json" -d '{"id": "work", "name": "Work"}'
curl -X POST $api/admin/v1/projects/work/keys -H "$json" -d '{"name": "laptop"}'
curl -X POST $api/admin/v1/nodes -H "$json" -d '{"id": "local"}'
curl -X POST $api/admin/v1/nodes/local/seats -H "$json" -d '{
  "id": "static", "engine": "static", "account": "static",
  "serves": [{"model": "echo-1"}], "config": {"text": "hello from voice"}}'
```

The key call answers a `"secret": "vk-..."` once: keep it. The node call answers an `"enrollment_token": "vn-..."`; start voice's node with it (`docker compose ls` names the compose project the habitat made for voice's services):

```sh
VOICE_ENROLLMENT_TOKEN=vn-... docker compose -p <project> up -d node
```

To serve a real model, give the seat an engine that reaches it (an `openai-compatible` seat pointed at Ollama on the host, for one) in place of `static`; voice's README lists the engines.

## 6. Point the organism's model map at it

The model map says where an organism asks for a kind of model. Point the `think.standard` category at voice, so the organism's own legs ask through it:

```sh
hale habitat model-map voice think.standard echo-1 --endpoint http://localhost:8080/v1/chat/completions --key-vault <slot>
hale habitat model-probe voice think.standard
```

The key is only ever a vault slot's name (store the `vk-...` secret there first, `hale dna secret set <NAME>` from voice's directory, and name that slot). `model-probe` asks the category's model once, where the map says it is asked, and prints the answer.

## 7. Ask it something

A streamed request, straight to voice:

```sh
curl -N http://localhost:8080/v1/chat/completions \
  -H 'Authorization: Bearer vk-...' -H 'content-type: application/json' \
  -d '{"model": "echo-1", "stream": true, "messages": [{"role": "user", "content": "hi"}]}'
```

The answer arrives as server-sent events, one `data: {...}` chunk after another (`chat.completion.chunk`), and ends with `data: [DONE]`. The organisms screen shows voice with its purpose and its pulse, and `GET /admin/v1/usage` on voice lists the request it just served.

## When it does not work

- `hale habitat status` names what is down; `hale habitat down` stops the body and keeps the data, and `up` brings it back.
- `graft` refuses a revision whose files are not the pinned ones, and a graft that stopped part way goes on where it stopped when you run it again.
- A host that answers with another host key is refused by `probe`; a `pending-key` host is waiting for the line in `authorized_keys`.
- An agent can work with the habitat the same way a person does: `hale habitat brief` prints what it needs to know.
