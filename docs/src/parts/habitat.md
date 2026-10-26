# Part VI: Multi-Record Control Plane (Habitat)

While a single DNA organism manages one application and its PostgreSQL/Git record, **Habitat** (`hale habitat`) is the control plane for provisioning, isolating, and connecting **multiple organisms** on a developer laptop or across remote SSH hosts.

## What Habitat Manages

- **Shared Backing Infrastructure (`hale habitat up` / `down`):** Starts and supervises the local Docker Compose stack (PostgreSQL, NATS JetStream, Prometheus, Forgejo, and the habitat's own `body` and `face` services).
- **Grafting & Provisioning (`hale habitat graft`):** Births new organisms (from a nursery sapling or pack), allocates isolated PostgreSQL schemas (`dna_<id>`), NATS streams (`DNA_<ID>`), Vault secret directories, and deterministic port blocks, and brings all unit services online.
- **Multi-Record Isolation & Deliberate Sharing:** Keeps each organism's database role, NATS subjects, and secrets isolated by default while providing explicit cross-record `connect` and `handoff` protocols.
- **Remote Host Execution (`hale habitat host`):** Registers SSH targets so `hale habitat graft <name> --on <host>` provisions and supervises organisms on remote Linux servers.

## Chapter in Part VI

- **[Habitat Architecture & CLI (`hale habitat`)](../habitat.md)** — Complete guide to `hale habitat` commands, resource allocation, multi-record security boundaries, and current implementation status.
