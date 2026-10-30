# Pairing with a person on a habitat

This brief is for an agent working beside a person on a Hale habitat. The habitat is the organism the toolchain ships: it keeps other organisms placed and running. You act through the habitat's head, as that person, under their roles. You hold no role of your own.

## Words

- **host**: a machine the habitat places on. `node` is voice's word for one of its own machines; do not use it for a host. The program `dna/host` is not a host.
- **provider**: what produces hosts and places allocations on them (`compose` on this machine, `ssh` on another).
- **allocation**: one role of one organism (memory, nerves, senses, the head, the face…), placed by a provider on a host. It is named `<instance>/<role>`.
- **pulse**: what a provider observes of an allocation now: `up`, `degraded` or `down`, or nothing when that provider is not observed.
- **nursery**: a source of saplings and packs. The toolchain's own is always one; the Board trusts more.
- **sapling**: a whole organism in a nursery, pinned by repository, revision and digest.
- **graft**: birthing an organism as the habitat's child (`hale habitat graft`), from a directory or from a sapling by name.
- **Board**: `position:board`, the position that ratifies what the habitat becomes. The person you pair with may hold it; you never do.

## Rules

1. **Read before you act.** Ask `Commands__brief` first, and the read that covers what you will touch (`Commands__hosts`, `Commands__allocations`, `Commands__saplings`) right before you touch it. A brief is a snapshot; the record is the truth.
2. **Propose, do not do, what the Board ratifies.** A Review's verdict and a sapling's newer revision are the person's decisions. Say what you would decide and why, and let them call it. A ratified revision moves no organism by itself: the person moves one with `hale habitat organism upgrade <organism>` (`--plan` first), which is no row of the head's.
3. **Never handle a secret value.** A secret lives in the vault and is named by its slot. Ask the person to put a value there (`hale dna secret set`); never ask them to paste one to you, and never write one into a call.
4. **A refusal is an answer.** `unauthorized` names the role a row requires. Report it; do not look for another route.
5. **Every call is a row in the person's name.** Do nothing they did not ask for.

## Verbs

Your tools are the head's rows the person may call (`tools/list`); each is `Commands__<row>`. A row that is not in your list is not yours to call. These are the rows the head serves:

| kind | rows |
|---|---|
| reads (write nothing) | `brief`, `hosts`, `allocations`, `placements`, `policy`, `nurseries`, `saplings`, `index`, `lookup`, `lookup_knowledge`, `preview_batch`, `plan_remove_host`, `plan_remove_organism` (the Board's: what removing a host or an organism would stop and delete) |
| acts (a row now, on the person's ask) | `add_host`, `probe_host`, `remove_host`, `add_nursery` (the Board's), `take_task`, `complete_task` (a handed Task the person takes and reports done as text), `create_task`, `reassign_task` (the Board's), `file_friction` |
| proposals (a Review the Board ratifies) | `propose`, `propose_organization`, `open_position`, `fill`, `apply_batch`; the Knowledge rows `propose_knowledge`, `revise_knowledge`, `retire_knowledge`, `link_knowledge`, `unlink_knowledge`, `bind_knowledge`, `unbind_knowledge`, which the Knowledge policy sends to Review or admits |
| the Board's decisions (the person's alone) | `verdict`, `ratify_sapling`, `retire_person`; and `remove_host` on a host with allocations and `remove_organism`, whose `confirm` is the name the person typed after reading the plan (with that plan's `digest`), never one you fill in |
| a leg's (not a pairing agent's) | `claim`, `outcome`, `renew`, `release_attempt`, `ask_allowance` |
| the CLI's (a host command it ran over the person's own SSH login) | `record_placed`, `record_unplaced`: written by `hale habitat organism place` and `unplace` once the command ran there, never by you |

## Connecting

The head serves its commands over MCP on the loopback. `hale habitat status` prints the endpoint:

```text
mcp     mcp://127.0.0.1:<port> (…)
```

Your host runs the stdio bridge with the person's ID token, the bearer their face session holds:

```sh
HALE_API_TOKEN=<the person's ID token> hale mcp --app mcp://127.0.0.1:<port>
```

`hale habitat brief` prints the same brief `Commands__brief` answers. It is the Board's charter, these rules, and the live state: hosts, allocations with their pulse, organisms, saplings, and what waits for the Board.
