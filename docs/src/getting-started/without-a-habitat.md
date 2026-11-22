# Running Without a Habitat

A Hale program compiles to one native binary. Running it needs that binary and the system libraries it links, and nothing else: no daemon, no control plane, no database. The habitat ([Part VII](../parts/habitat.md)) is for DNA organisms. It is not a prerequisite for programs or services, and nothing on this page uses it.

This page takes one small HTTP service, `notes.hl` (section 2), from source to a process under systemd or in a container.

---

## 1. Run and Build

`hale run` compiles and runs in one step. Arguments after the target go to the program:

```sh
hale run notes.hl                    # compile to a temporary binary and run it
hale run notes.hl 127.0.0.1:8081     # the same, with the program's own argv
```

`hale build` writes the binary to disk, beside the source unless `-o` says where:

```sh
hale build notes.hl                                     # ./notes, tuned for this machine's CPU
hale build --target-cpu portable notes.hl -o out/notes  # for any machine of this architecture
./notes
```

Build with `--target-cpu portable` for anything you ship to another machine, such as a server or a container image. The default tunes for the build host's CPU, so the binary may not start on an older one. A directory works as a target too (`hale build .`), and builds the directory's `.hl` files as one program.

On Linux, the binary links the C library, OpenSSL (`libssl`, `libcrypto`) and zlib dynamically. The machine that runs it needs those, and does not need `hale`, `clang` or LLVM.

---

## 2. A Service: `notes.hl`

This program serves an API over HTTP, takes its configuration from argv and the environment, and exposes health probes and Prometheus metrics:

```hale
type AddNote { text: String; }
type Added { id: Int; count: Int; }
type Count { count: Int; }

locus Notes {
    params { count: Int = 0; }
    fn add(req: AddNote) -> Added {
        self.count = self.count + 1;
        return Added { id: self.count, count: self.count };
    }
    fn total() -> Count {
        return Count { count: self.count };
    }
}

api NotesApi {
    rpc Notes::add;
    rpc Notes::total;
}

// The bearer a caller presents, checked against the one the operator configured.
locus Token {
    params { want: String = ""; }
    fn principal(token: String) -> std::api::Principal {
        if len(self.want) > 0 && token == self.want {
            return std::api::Principal { mode: "bearer", name: "operator" };
        }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

main locus App {
    params {
        bind: String = "127.0.0.1:8080";
        token: String = "";
        probe_port: Int = 9090;
        metrics_host: String = "127.0.0.1";
        metrics_port: Int = 9100;
        notes: Notes = Notes { };
        auth: Token = Token { want: self.token };
        web: api::Exposure = api::serve(
            NotesApi,
            http::Rpc { bind: self.bind, codec: json, principals: self.auth },
            as: "web",
            receivers: { Notes: self.notes },
            bound: 64,
            on_full: refuse
        );
        probes: std::observe::Probes = std::observe::Probes { port: self.probe_port };
        metrics: std::observe::Scrape = std::observe::Scrape {
            host: self.metrics_host,
            port: self.metrics_port,
        };
    }
    placement { metrics: pinned; }
}

fn main() {
    let cfg = std::cli::Resolver { env_prefix: "NOTES_", argv_keys: "bind\n" };
    App {
        bind: cfg.get("bind", "127.0.0.1:8080"),
        token: cfg.get("token", ""),
        probe_port: cfg.get_int("probe_port", 9090),
        metrics_host: cfg.get("metrics_host", "127.0.0.1"),
        metrics_port: cfg.get_int("metrics_port", 9100),
    };
}
```

`App` writes no `run()`. A `main locus` that holds an exposure gets a generated one that waits on it, so the process serves until it is stopped. [Build a Typed API](../services/build-an-api.md) builds a larger service step by step, and [API Surfaces](../services/api.md) covers every transport (Unix sockets, REST, MCP, gRPC, WebSockets).

---

## 3. Configuration: Flags and Environment

`std::cli::Resolver` reads each setting from a positional argument, then from an environment variable named with its prefix, then from the default given in the source ([CLI Arguments & Environment](../everyday/cli-config.md)). In `notes.hl`, `bind` is the first argument or `NOTES_BIND`. Every other setting comes from its variable only (`NOTES_TOKEN`, `NOTES_PROBE_PORT`, `NOTES_METRICS_HOST`, `NOTES_METRICS_PORT`), because `argv_keys` names only `bind`:

```sh
NOTES_TOKEN=s3cret ./notes 127.0.0.1:8080
NOTES_TOKEN=s3cret NOTES_BIND=0.0.0.0:8080 NOTES_METRICS_HOST=0.0.0.0 ./notes
```

The program resolves its configuration once, in `fn main()`, and hands the values to `App` as params. A param default can read an earlier param (`Token { want: self.token }`, `port: self.probe_port`), which is how one setting reaches a probe port or a listener.

The runtime reads a few variables of its own. `HALE_LOG` and `HALE_LOG_FORMAT` (`logfmt`, `json`, `text`) set the level and format of `std::log` records and of the runtime's own diagnostics on stderr ([Structured Logging](../everyday/logging.md)). The `LOTUS_*` knobs tune queues, drain and diagnostics ([Production Operations](../systems/operations.md)).

---

## 4. Calling It

With the service running, `hale describe` lists what a caller may call, and `hale call` calls one member. Both take the bearer as `--bearer` or from `HALE_API_BEARER`:

```sh
hale describe http://127.0.0.1:8080 --bearer s3cret
hale call http://127.0.0.1:8080 Notes::add --text hello --bearer s3cret
HALE_API_BEARER=s3cret hale call http://127.0.0.1:8080 Notes::total '{}'
```

```text
NotesApi@fnv1a64:487243cf2786f3cb/web  [http 127.0.0.1:8080]
caller: bearer operator; roles: none
  Notes::add(text: String) -> Added  requires: -
  Notes::total() -> Count  requires: -
{"id":1,"count":1}
{"count":1}
```

A caller with the wrong bearer is refused `unauthenticated`, with the reason `refused()` gives; one with no bearer is refused `unauthenticated` as well, and with `NOTES_TOKEN` unset every caller is. Any HTTP client works too: a call is `POST /call/<member>` with the bearer in `Authorization` and the request as its JSON body ([API Surfaces](../services/api.md)):

```sh
curl -X POST -H 'Authorization: Bearer s3cret' -d '{"text":"hello"}' http://127.0.0.1:8080/call/Notes::add
```

---

## 5. Observability Endpoints

`notes.hl` declares two of the three observation backends on its `main locus` ([Observability & Probes](../everyday/observing.md)):

| Endpoint | Backend | Address in `notes.hl` |
|---|---|---|
| `GET /healthz`, `/readyz`, `/capacity` | `std::observe::Probes` | `0.0.0.0:9090` (`Probes` binds every interface) |
| `GET /metrics` (Prometheus text) | `std::observe::Scrape` | `127.0.0.1:9100` by default; `NOTES_METRICS_HOST=0.0.0.0` to scrape from elsewhere |

```sh
curl http://127.0.0.1:9090/readyz      # ready
curl http://127.0.0.1:9090/capacity    # {"ready":true,"draining":false,"queues":[...]}
curl http://127.0.0.1:9100/metrics     # hale_loci_live, hale_process_resident_memory_bytes, ...
```

`/readyz` answers `503 starting` until `App`'s children are born, `200 ready` while it serves, and `503 draining` once it stops. `std::observe::Otlp` adds OpenTelemetry export of lifecycle events. A program that declares none of the three opens no observation port.

---

## 6. Stopping

`SIGTERM` and `SIGINT` drain the program: the server answers the calls already in flight, `drain()` and `dissolve()` run on every locus, deepest first, and the process exits `0`. If the drain has not finished after 5 seconds (`LOTUS_DRAIN_GRACE_MS`), the runtime logs what is stuck and the signal ends the process. A supervisor's stop timeout should be longer than that grace ([Locus Lifecycle](../services/lifecycle.md)).

---

## 7. Under systemd

Install the binary (built with `--target-cpu portable` if it was built elsewhere) and give it a unit. Keep the token out of the unit, in a file only root reads (`/etc/notes/env`, mode `600`, holding `NOTES_TOKEN=…`):

```ini
# /etc/systemd/system/notes.service
[Unit]
Description=notes API
After=network-online.target
Wants=network-online.target

[Service]
Type=exec
ExecStart=/usr/local/bin/notes
Environment=NOTES_BIND=127.0.0.1:8080
EnvironmentFile=/etc/notes/env
DynamicUser=yes
Restart=on-failure
TimeoutStopSec=15

[Install]
WantedBy=multi-user.target
```

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now notes
systemctl status notes
journalctl -u notes
```

`systemctl stop` sends `SIGTERM`, the program drains, and the unit ends `Result=success`. Runtime diagnostics go to stderr, and so to the journal.

---

## 8. In a Container

The published toolchain image, `ghcr.io/hale-lang/hale:<version>`, builds the binary. The image that runs it needs only the libraries it links:

```dockerfile
FROM ghcr.io/hale-lang/hale:0.38.0 AS build
WORKDIR /src
COPY notes.hl .
RUN hale build --target-cpu portable notes.hl -o /out/notes

FROM ubuntu:24.04
RUN apt-get update \
 && apt-get install -y --no-install-recommends libssl3t64 zlib1g \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/notes /usr/local/bin/notes
ENV NOTES_BIND=0.0.0.0:8080 NOTES_METRICS_HOST=0.0.0.0
EXPOSE 8080 9090 9100
USER nobody
ENTRYPOINT ["/usr/local/bin/notes"]
```

```sh
docker build -t notes .
docker run -d --name notes -p 8080:8080 -p 9090:9090 -p 9100:9100 -e NOTES_TOKEN=s3cret notes
docker stop notes     # SIGTERM: the program drains and exits 0
```

A listener on `127.0.0.1` is not reachable through a container's published port, so the image sets `NOTES_BIND` and `NOTES_METRICS_HOST` to bind `0.0.0.0`. Point the orchestrator's health and readiness checks at `/healthz` and `/readyz` on port 9090. `hale deploy render <seed> --backend compose|kubernetes` writes a starting Compose file or Kubernetes manifest from the program itself, with those probes wired in ([Multi-Binary Systems & Deployment](../services/multi-binary.md)).

---

## When You Want a Habitat

Everything above is a process you run and supervise yourself. A habitat (`hale habitat up`) is the control plane for **DNA organisms** ([Part VI](../parts/organism.md)): applications whose changes go through positions, reviews and a signed record. It runs as a Docker Compose project, on this machine or another reached over SSH. It grafts each organism from itself, and gives it a Postgres for its record and memory, NATS for its events, Prometheus, a forge, and credentials of its own. It places the organism's body on the hosts that join it, and keeps all of it running.

Reach for one when you want that: an organism, or several sharing infrastructure the habitat provisions and keeps apart. To run a program or serve an API, the binary is enough. [The Habitat](../habitat.md) covers the rest.

Next: [Part I: The Language](../parts/language.md).
