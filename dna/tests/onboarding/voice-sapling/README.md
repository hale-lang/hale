# voice, as a sapling stands in for it

The acceptance fixture (`dna/tests/acceptance_homelab_test.hl`) grafts this
tree as the sapling `voice` from a local git nursery: CI fetches nothing.

It keeps the shape of hale-lang/voice where the habitat touches it:

- a repository whose `hale.toml` declares the roles it needs at graft;
- a `compose.yaml` at its root: the services the habitat places on a host
  and applies there;
- an OpenAI-compatible `/v1/chat/completions` that answers a streamed
  request as server-sent events;
- a seat that relays to a local model reached over the host's network (on
  the real box, Ollama on the host);
- the habitat's models: `[dna.provides.models]` names its service and the
  drawn family `tenant-key` its tenants are keyed by. The habitat draws a
  member per organism into voice's reader vault (`HALE_VAULT_DIR`, the
  members listed by `HALE_SECRET_TENANT_KEY_FAMILY`), and voice answers
  `GET /v1/models` and a chat only with a member as the bearer token.

What it does not keep: voice's own programs (its api, brain and node, built
by the toolchain into its image), its store, its broker, its accounts and
keys, and the Mac's launchd node. The fixture says so where it stops.
