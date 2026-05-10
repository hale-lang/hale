# Deployment

## Synopsis

Deployment binds bus subjects to transports — the same Aperio
source can run as a single in-memory process or as multiple
binaries communicating over Unix sockets, NATS, UDP multicast,
or TCP. The binding is configured at startup, not in source.

## Configuration sources

Two configuration surfaces exist:

| Surface | Status | Purpose |
|---|---|---|
| `LOTUS_BUS_CONFIG` env var | **v0 — implemented** | Line-oriented config file, consumed by the runtime |
| `deployment.yaml` | Roadmap | Higher-level YAML config with glob patterns and per-transport options |

For v0, only the env var format is wired up. The
`deployment.yaml` shape is documented for the example tree and
will be implemented in a future runtime revision.

## `LOTUS_BUS_CONFIG` (v0)

The env var points at a small line-oriented config file:

```text
trellis.observation = unix:///tmp/trellis-obs.sock    : listen
trellis.kernel      = unix:///tmp/trellis-kernel.sock : connect
```

Each line:

```text
<subject> = <transport-url> : <role>
```

| Field | Forms |
|---|---|
| `<subject>` | A bus subject the binary subscribes to or publishes on |
| `<transport-url>` | `unix://<path>` (v0) |
| `<role>` | `listen` or `connect` |

### `listen` vs `connect`

| Role | Behavior |
|---|---|
| `listen` | Bind the transport endpoint; accept incoming connections from peers (background thread) |
| `connect` | Connect to the endpoint as a client; retry until the listener is up |

For a given subject, exactly one process should be in `listen`
role; one or more processes can be in `connect` role.

### Multi-peer fanout

Multiple `connect` lines on the same subject pointing at
different listeners produce fan-out — the publisher delivers
each message to every peer:

```text
evt = unix:///tmp/peer-a.sock : connect
evt = unix:///tmp/peer-b.sock : connect
```

### Unconfigured subjects

Subjects the binary uses but does not list in
`LOTUS_BUS_CONFIG` fall back to in-process dispatch. A binary
started with no `LOTUS_BUS_CONFIG` at all behaves identically
to a single-process program — every subject is in-memory.

## `deployment.yaml` (roadmap)

The intended future surface, as shown in the example tree:

```yaml
channels:
  "trellis.observation":
    transport: udp_multicast
    group: "239.7.7.7"
    port: 9000

  "trellis.kernel":
    transport: nats
    url: "nats://nats-control:4222"

  "trellis.action":
    transport: nats
    url: "nats://nats-control:4222"

  "trellis.receipt":
    transport: nats
    url: "nats://nats-control:4222"
```

Glob patterns for testing:

```yaml
channels:
  "trellis.*":
    transport: in_memory
```

A binary whose deployment binds every `trellis.*` subject to
the in-memory transport runs as a single-process integration
test, with no source-level changes.

### Transport options (planned)

| Transport | Options |
|---|---|
| `in_memory` | (none) |
| `unix_socket` | `path` |
| `nats` | `url`, `creds_file`, `tls` |
| `udp_multicast` | `group`, `port`, `ttl` |
| `tcp` | `address` (`host:port`) |

## Schema agreement

Schema agreement is by *compilation*, not by runtime
negotiation. Both binaries that share a subject must compile
the same `type` declarations from the same source. The wire
format is exactly the in-memory layout; no headers, no
versioning, no schema description on the wire.

For schema evolution across deployments, the perspective
versioning mechanism (`serialize_as TypeV1`) is roadmap. v0
does not support rolling deployments with mismatched schemas.

## Environment variables

| Var | Purpose |
|---|---|
| `LOTUS_BUS_CONFIG` | Path to the line-format bus config file |

## See Also

- [Bus dispatch](./bus/index.md)
- [Runtime](./runtime.md)
- [Memory model](./memory.md)
