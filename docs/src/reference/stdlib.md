# Standard library

The stdlib lives in two shapes:

- **Path-call dispatch** — `std::env`, `std::time`, `std::str`,
  `std::io::fs`, `std::process`, `std::ts`, ... — these route
  directly to a libcall in the C runtime; there's no `.ap`
  source backing them.
- **Namespace lotus** — `std::cli`, `std::iter`, `std::json`,
  `std::lang`, `std::log`, `std::yaml`, `std::text`,
  `std::io::tcp`, ... — these are written in pure Aperio under
  [`crates/aperio-codegen/runtime/stdlib/*.ap`](https://github.com/local/lotus-lang/tree/main/crates/aperio-codegen/runtime/stdlib).

Until each module gets a dedicated reference page, the canonical
documentation for any stdlib path is the `.ap` source itself
(for namespace-lotus modules) or
[`spec/stdlib.md`](https://github.com/local/lotus-lang/blob/main/spec/stdlib.md)
(for the path-call surface).

The integration tests under
[`crates/aperio-codegen/tests/`](https://github.com/local/lotus-lang/tree/main/crates/aperio-codegen/tests)
exercise every documented stdlib path; reading them is a fast
way to see what each module expects.
