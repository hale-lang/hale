# api-bench: a served call, measured

`echo.hl` serves one surface (`Echo::ping`, a record in and a record out)
over `unix::Rpc` (`/tmp/hale-bench-api.sock`) and `http::Rpc`
(`127.0.0.1:18431`). `client.py` drives it from Python, one connection,
and prints the per-call p50 and p99.

```sh
hale build scripts/api-bench/echo.hl -o /tmp/echo-a     # one compiler
hale build scripts/api-bench/echo.hl -o /tmp/echo-b     # the other
taskset -c 2 /tmp/echo-a & P=$!; sleep 1.5
taskset -c 4 python3 scripts/api-bench/client.py unix 100000
taskset -c 4 python3 scripts/api-bench/client.py http 50000
grep VmRSS /proc/$P/status; kill $P
```

For an A/B, build both binaries the same way (`hale build`, the default
O3), run them interleaved over several rounds with the server and the
client pinned to different cores, and compare medians over the rounds: a
round's p50 moves by a few microseconds on its own. Read the resident set
before and after a run for memory per call (`/proc/<pid>/status`,
`VmRSS`). `crates/hale-codegen/tests/api_serve_memory.rs` holds the memory
bound per transport in CI.
