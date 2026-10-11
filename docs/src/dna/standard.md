# Held together by the compiler

> **This chapter is a standard, and partly a goal.** It says how a DNA project uses the whole toolchain, one way for each concern, so that the compiler can hold a distributed system together. Every rule says what it is, why, what it replaces, how it is enforced, and where DNA itself stands today. A rule marked *goal* is not yet followed by DNA's own source; the list of those is the migration this chapter commits to.

Most of Hale's features are opt-in and orthogonal: a program can use topics without surfaces, claims without effects, units without either. That is right for a language, and wrong for an organization. A system stays coherent only if every part makes the same choices, so that the compiler can see the whole of it: which processes exist, what crosses between them, what each part may reach, what a value means. This chapter makes those choices once.

An organism is a distributed system: the record and its memory, a head, nodes, legs, an application with its heart, a broker for the nerves, a vault, the senses. The aim is that everything which joins those parts is a declaration the compiler checks, not a convention people remember.

## The rule behind the rules

**Anything that crosses a boundary is declared, and anything declared is checked.** A boundary is a process, a part, an authority, a unit of measure or a secret. The rest of the chapter applies that rule to each concern.

## Structure: one tree, one plan

**A program is one locus tree.** Each part of an organism is a locus with its children as params; work that comes and goes is a child born by `accept` and ended by `dissolve`; failure goes to the owner's `on_failure` (`spec/semantics.md` § Locus instantiation).
- *Why:* ownership is the one structure the compiler, the runtime and the memory model all share.
- *Instead of:* processes, threads or registries kept beside the tree.
- *Today:* followed. The organization is one tower (`dna/core/assembly.hl`'s `Dna`), and an execution's workflows, tasks and steps are accepted children.

**Every process is an instance of a fleet plan, and every deployment is derived from it.** The organism's programs (the host, the organization, the head, the application, the reflexes) are instances of one plan, their routes the topics they bind; `hale fleet check` composes them. A deployment backend renders the plan for the platform it runs on (GH #1477).
- *Why:* only a plan lets the compiler check what crosses between processes, and only a derived deployment cannot drift from the program.
- *Instead of:* hand-written compose files, pid files and ports repeated in several places.
- *Enforced by:* `hale fleet check` in verification, on every candidate.
- *Today:* **goal.** The organism has no plan of its own; its bindings are copied by hand between the host, the generated organization and the reference organism, and its processes are started by the host.

**Placement is declared on the main locus.** Pools, pinning and replicas are `placement { }`, never threads or process flags.
- *Today:* followed by the host, the head, the reflexes and the generated organization.

## Boundaries: declared, typed, generated

**Every call between processes is an `api` surface.** A program serves a surface with `api::serve` over the transport the deployment chooses (`unix::Rpc`, `http::Rpc`, `mcp::Rpc`), with roles on its rows; callers use clients generated from its description (`hale api client`), checked current in CI.
- *Why:* the surface is the contract: its description is what `hale api describe` reads, what an agent's tools are rendered from, and what a change is diffed against.
- *Instead of:* hand-written HTTP routes that build JSON field by field, and hand-written clients that read it back.
- *Today:* the head's commands are a surface (`HeadCommands`, served over a socket and HTTP) with a generated client for the legs, checked by its digest. The head's **reads are a goal**: every read route, the project head's operations and the face's client are written by hand.

**Every event between parts is a declared topic, bound through an adapter.** A topic names its payload type and subject; a main locus binds it (`bindings { T: Adapter }`), and keyed subscriptions say whose events a locus hears.
- *Instead of:* raw broker connections and subject strings.
- *Today:* followed inside the organism (about fifty topics in `dna/core/topics.hl`). **Goal:** the heart. An application's events reach the organism untyped, and the todo application publishes to the broker over a raw connection, because its public library lacks a credentialed publisher.

**A surface's machine forms are generated and committed, and a test holds them current.**
- *Today:* followed by the todo application (its OpenAPI, MCP, description and JSON Schema forms, and its TypeScript client).

**A handler's failure is declared.** A surface handler or an interface method that can fail says `fallible(E)`, so a failure is the transport's error outcome, never a success carrying a code.
- *Today:* **goal.** The head's command handlers return `{ok, code}`; interface methods cannot be fallible yet (`dna/FRICTION.md` F.7).

## Data: one meaning per value

**Every row of the record is a declared type.** A row kind is a type with `json` tags; it is written and read through its generated codec, and the routing table names types, not strings.
- *Why:* the record is the organism's memory, and a field name typed twice is a bug the compiler cannot see.
- *Instead of:* building row bodies with a JSON builder and reading them field by field.
- *Today:* **goal.** Rows are strings, and their fields are read by hand in over a thousand places.

**A closed set is an enum.** *Instead of:* a `String` with its allowed values in a comment. *Today:* **goal** (one hand-written enum).

**Money is `Decimal` with its currency; time is `Time` and `Duration`; any other measure is a quantity in its unit.** An integer quantity (`quantity Int in …`) is the choice where performance demands it, never a bare `Int` (`spec/units.md`).
- *Why:* the unit is part of the value's meaning, and the compiler can only check what it is told.
- *Today:* **goal.** Spend is integer micro-dollars and times are integers with their unit in the name.

**Lists are collections, not delimited strings.** A list that crosses a boundary is a `bounded[T; N]` where its size is known, or a collection locus where it is not.
- *Today:* **goal**, and partly a language question: types cannot hold unbounded collections (`notes/value-collections.md`), so lists travel as newline-joined strings.

## Authority: law, effects and secrets

**Every program carries a constitution, over a base every environment shares.** `hale.toml` names a base (`[claims] base = "…"`) and each environment's constitution; the organism's own law (`dna/org/law.hl`) is the base.
- *Instead of:* `no_base`, and programs with no law at all.
- *Today:* the generated organization has its law, but under `no_base`; the host, the head, the reflexes and the face carry none. **Goal.**

**Every place a program touches the operating system names its purpose.** `require attributed(all syscall)` in the base: each function that performs a syscall directly carries a user-declared effect class (`spec/verification.md` § Claims).
- *Why:* a purpose can be claimed over (`forbid reaches`), reviewed and narrowed.
- *Today:* **goal.** The organism declares its effect classes (`dna/core/types.hl`) and annotates its carriers, but the programs that run it declare none.

**A projection is a pure function.** Every projection of the record is `@no_syscall` (and `@deterministic` where it is), so replay gives the same answer.
- *Today:* one projection is; the rest are **goal.**

**A secret is held only by a sealed locus.** Through `std::secret`, revealed only in the statement that writes it to a wire.
- *Instead of:* a secret as a `String` from the environment, or passed to a child process.
- *Today:* followed for every model key and broker password. **Goal:** provisioning (the vault's own writes, a database role's password, the forge token, an application's bearer table), which the standard library does not cover yet.

**Authority belongs to principals, and a backend's policy is a projection of it** (`runs_under`, [The habitat, as designed](../habitat.md)). *Today:* not built.

## Resources: bounded by design

**Placement and capacity are declared, and a growing allocation says what bounds it.** A hot path declares its budget (`@budget`), a buffer its form (`@form(ring_buffer)`, a `capacity` slot), and `@unbounded` is written only where the allocation summary (`hale check --dump-alloc-summary`) shows a real site, with the input that bounds it named beside it.
- *Why:* an annotation that silences nothing teaches nothing, and an organism's processes run for months.
- *Today:* **goal.** DNA carries about 850 `@unbounded` outside its tests, and about two thirds of them suppress nothing: they were copied with refactored code because the verification gate fails on any advisory. The rest are mostly analysis imprecision (a string built in a loop and returned; a replaced `String` field); a few dozen mark real growth bounded by the record's length, which the language cannot yet say.

**A program says when it is healthy and how full it is.** The runtime serves liveness, readiness and capacity from what it already knows, with the application's own readings added, and the deployment backend maps them to the platform's probes and scaling (GH #1477). *Today:* not built.

## Quality: the toolchain is the gate

**Every candidate is judged by the toolchain, and the judgment is read as data.** Verification runs `hale fmt --check`, `hale check`, `hale verify`, `hale test`, `hale fleet check` and `hale model diff` on every change and reads their JSON output.
- *Instead of:* scraping text (a test count in prose) or deciding a topology change by file name.
- *Today:* the gate is followed (`dna/core/verification.hl`); **goal:** reading `hale test --json`, and judging topology from the model diff's facets.

**`hale verify` covers all of the organism's source.** *Today:* it covers `dna/core`; the rest is **goal.**

**A surface is tested through its fixture transport.** `std::api::test::Rpc` drives the surface as a caller would; a test never builds a call context by hand. *Today:* followed by the todo application; **goal** for the head's tests.

**Model calls in tests are scripted or replayed, never live.** `FakeModel` (rules, answers, turns) for mechanics, `RecordedModel` tapes for replay; a live model is a deliberate evaluation, not a test.

## Observability: one channel

**Readings are declared, and the organism and the platform read the same ones.** A series is declared beside the code that moves it, served by `std::metrics`, and is the same endpoint a deployment's probes read.
- *Instead of:* series names as string literals, and two channels (the senses and the inspector) that never meet.
- *Today:* **goal.** The senses and `LOTUS_OBS` are separate, and the todo application serves no readings.

## Layout

- **A part is a seed** (a directory of `.hl` with its `hale.toml`), with its tests in `*_test.hl` beside it or under its `tests/`.
- **Contracts live in `spec/`**, generated from the surfaces and committed.
- **The organism's own source is `dna/org/`**, generated by `init` and changed only through reviewed changes; `vendor/dna` is the toolchain's and is never edited by hand.
- **Configuration is typed params on the main locus**, bound per environment in `hale.toml`; an environment variable is read in one place, at the edge.

## What follows from it

Followed, these rules make a change to the organism checkable before it runs: a new topic that no instance binds, a surface field whose type changed under a caller, a function that reaches a secret or the network without its purpose, a deployment that no longer matches its program, a money value passed where a duration was meant. Each is a build error with a path through the source, not an incident.

The *goal* rules above are DNA's own migration, in the order they tighten the system most: typed rows; the organism as a fleet plan, with its broker permissions derived from its bindings; the head's reads as surfaces with generated clients; a typed heart; law and attributed effects on every program; verification reading data; units for money and time; enums; secrets provisioned on the sealed side.
