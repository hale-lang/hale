# Friction

What the todo repository ran into on the toolchain it was written against (hale main after #1451), in the order met. Each is a finding for the toolchain, not for this repository; the workaround used here is named with it.

1. **A hub is bound only to its own seed.** A topic, publishing locus or surface declared in an imported seed is refused: "no locus publishes `lib::Ticks`", "no surface `lib::Svc` is declared", and `ws::Hub` fails with "`ws` is not an import or a type of this seed". Importing an app that has a hub also fails. Reproducers: `scratch/todo-scratch/repro-hub` (cross-seed, fails) and `repro-hub2` (local, with an unrelated import, passes). I worked around it with a flat seed and by importing single files (`../todo`, `../auth`, `../nerves`) from the tests.
2. **The rpc codec carries no list.** Both `bounded[T; N]` and arrays are refused (`crates/hale-types/src/surfaces.rs`, `json_refusal`). `list` and `overdue` therefore return `{count, json: String}`.
3. **`Time` has no JSON form.** `due` travels as text. `parse_time("2999-…")` wraps past the i64-nanosecond limit (2262), so `add` refuses years above 2200.
4. **`hale api call --id` is the verb's own flag**, so a payload field named `id` cannot be set from a flag. The request field is `todo`, but the item record still has an `id`.
5. **The surface `Todo` and a type `Todo` collide in the `.proto`** ("the message `Todo` is named twice"), so the record is `TodoItem`.
6. **`http::Rpc` answers no CORS preflight** (`OPTIONS` → 400 malformed), so a page on another origin cannot call the api. The hub serves the surface as call frames, and the page lists through that.
7. **The generator emits only TypeScript**, so a no-build page needs a `tsc` step. I could not run `tsc`, so no `todo.js` exists.
8. **Public pond has no `credential:` or `NatsPublisher`**; those exist only in the `dna/core/pond` fork. `vendor/dna` appears only after `init`, and `hale check` cannot resolve `vendor/dna/pond/…` before it. The sense therefore uses its own small NATS client, and the `Credential.reveal_text()` rule did accept `stream.send(…)` as a wire write.
9. **`hale check --workspace` merges every `.hl` in a directory into one seed**, so several `*_test.hl` files in `api/tests/` collide on `fn main`. Each test program lives in its own subdirectory (`sense/`, `silent/`, `contract/`, plus the shared `fake/`).
10. **`placement { }` is only valid inside a `main locus`**, and a direct method call on a locus on another pool is refused, so every test drives the list through `std::api::test::Rpc` and the bus.
11. **Pinned loci that block in `accept` and a `run()` that sleeps** hold a test process open, so the sense tests end with `std::process::exit(0)`.
