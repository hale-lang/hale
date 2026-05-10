# io-demo

Phase 1 capstone. Combines `std::io::fs` (config + log),
`std::io::tcp` (Listener), `std::env` (argv + env), and
`std::str::parse_int` (numeric argv) into a single Aperio
program that exercises the v1.x stdlib's first arc end-to-end.

## What it does

1. Resolve a port: `argv[1]` if it parses as a positive
   integer, otherwise `9876`.
2. Resolve a config path: `$APERIO_IO_DEMO_CONFIG_PATH`
   if set, otherwise `/tmp/aperio_io_demo_config.txt`.
3. Resolve a log path: `$APERIO_IO_DEMO_LOG_PATH` if set,
   otherwise `/tmp/aperio_io_demo_log.txt`.
4. If a config file exists at the resolved path, read its
   contents into the log payload; otherwise use a default.
5. Bind a TCP Listener on `127.0.0.1:<port>`, wait for one
   connection, log the accepted fd, close.
6. Write the log payload to the resolved log path. Print
   where it landed and exit.

## Run it

Default port and paths:

```
aperio run examples/io-demo/main.ap
nc 127.0.0.1 9876        # in another terminal
cat /tmp/aperio_io_demo_log.txt
```

Custom port:

```
aperio run examples/io-demo/main.ap 9000
nc 127.0.0.1 9000
```

Custom config path:

```
echo "custom payload" > /tmp/my-config
APERIO_IO_DEMO_CONFIG_PATH=/tmp/my-config \
  aperio run examples/io-demo/main.ap
```

## Primitives this exercises

- **`std::env::args_count`, `std::env::arg`,
  `std::env::var`, `std::env::var_exists`** — process-level
  state captured in main's prelude.
- **`std::str::parse_int`** — string-to-Int with sentinel
  fallback. The `parsed > 0` guard rejects both garbage
  input (returns 0) and explicit non-positive values.
- **`std::io::fs::file_exists`, `read_file`, `write_file`** —
  one-shot synchronous file ops.
- **`std::io::tcp::Listener`** — stdlib locus with the
  three-stage TCP lifecycle (`birth` binds, `run` accepts,
  `dissolve` closes).
- **Magic `std::*` paths** — every stdlib reference goes
  through the m71 path resolver. No `import`, no `use`.
- **Stdlib-loci-via-bundled-source** — the Listener's
  declaration lives in `runtime/stdlib.ap`, concatenated to
  this program at codegen time per the m73a mechanism.
- **`if` / `else` / `let mut` reassignment** — ordinary
  Aperio surface, exercised against stdlib return values.

## Phase-1 limitations this honestly inherits

- The Listener accepts exactly one connection then exits.
  Servers that handle many connections wait on the
  multi-accept arc (see `docs/std/src/io/tcp.md`).
- Binary payloads with embedded NULs would truncate at
  `write_file` time. UTF-8 strings only for v0.
- `parse_int` is base 10 only and doesn't trim whitespace
  (see `docs/std/src/str.md`).

## Integration test

`crates/aperio-codegen/tests/io_demo.rs` builds this example
and runs three scenarios, each on a freshly picked port +
unique /tmp paths so the tests are fully parallel-safe:

- Default config (no seeded file): logs the default payload.
- Seeded config: logs the file contents.
- Garbage argv[1] (`"not-a-port"`): falls back to port 9876.
