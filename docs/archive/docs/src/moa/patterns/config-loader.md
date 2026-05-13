# Config-loading orchestrator

The orchestrator's job under MOA is to compose the app from
configuration. When configuration parsing is non-trivial — multiple
flags, env vars, config files, validation — that's its own concern,
and the concern wants its own memory-owner.

## The shape

A small CLI memory-owner sits as a leaf in the orchestrator's child
set. Its state is the parsed config (typed fields, one per setting).
Its surface is method calls or contract exposure that lets the
orchestrator read the parsed values without reaching into argv
directly.

The orchestrator:

1. Instantiates the CLI memory-owner with raw argv (or default
   strings).
2. Reads typed config values via methods (`r.flavor()`,
   `r.depth_int()`) or via contract surface.
3. Instantiates the application memory-owners with the resolved
   config as their birth-time params.
4. Holds no config state of its own.

## Why factor out CLI

Three reasons keeping argv parsing out of `main()` is worth the
small extra locus:

- **Orchestrator stays stateless.** Property #3 (orchestrators
  carry no state) holds. If `main()` parsed argv into local mutable
  bindings, walked through validation, and held the resolved values
  across instantiation calls, it would be carrying state — small,
  but the property is binary, not graded.
- **Parsing logic is reusable.** Multiple binaries in the same
  domain (a trading platform with a fitter and an applier and a
  reconciler) likely share config conventions. A
  `MarketConfigResolverL` factored out once is consumed by each.
- **Testing surface.** A `MarketConfigResolverL` can be instantiated
  with synthetic argv in unit tests; you assert on its exposed
  values. Embedded argv parsing in `main()` is testable only via
  subprocess.

## stdlib::cli::Resolver

The stdlib ships a generic `std::cli::Resolver` (see
`docs/src/std/cli.md`) that handles the common shape:

```aperio
fn main() {
    let r = std::cli::Resolver { defaults: "seed=42;runs=2" };
    let seed = r.int_or("seed", 42);
    let runs = r.int_or("runs", 2);

    // application memory-owners get the resolved values:
    let _g = MdGatewayL { coord: "gateway", seed: seed };
    let _a = BookL { coord: "book.a" };
    let _b = BookL { coord: "book.b" };
}
```

`std::cli::Resolver` is a self-contained recording memory-owner.
Its state is the parsed argv defaults; its surface is the
`int_or` / `string_or` / `bool_or` accessor methods.

## When to write your own

Use `std::cli::Resolver` for typical flag-and-default needs. Write
a domain-specific Resolver when:

- Argv parsing requires non-trivial validation (consistency checks
  across flags; required-flag enforcement; flag dependencies).
- The config surface is large enough that bare key-value access is
  noisy (the binary has 30+ flags; a typed config struct is
  clearer).
- Config sources extend beyond argv — env vars, config files,
  remote-fetch. A domain Resolver layers those without forcing every
  caller to know which source provided which value.

In any of those cases, the domain Resolver is itself an
MOA-shaped memory-owner. Capacity is `params` (or a single struct
type if the config has many fields). Ingest is none — config
doesn't change after birth. Publish surface is typically none —
config is read, not broadcast.

## Worked example

A simulator that takes seed + scenario file + speed multiplier:

```aperio
locus SimulatorConfigL {
    params {
        seed: Int = 42;
        scenario_path: String = "scenarios/default.yaml";
        speed: Float = 1.0;
    }

    fn seed() -> Int { return self.seed; }
    fn scenario_path() -> String { return self.scenario_path; }
    fn speed() -> Float { return self.speed; }
}

fn main() {
    let r = std::cli::Resolver { defaults: "seed=42;speed=1.0" };
    let cfg = SimulatorConfigL {
        seed: r.int_or("seed", 42),
        scenario_path: r.string_or("scenario", "scenarios/default.yaml"),
        speed: r.float_or("speed", 1.0),
    };

    let _scenario = ScenarioL {
        seed: cfg.seed(),
        path: cfg.scenario_path(),
        speed: cfg.speed(),
    };
}
```

`SimulatorConfigL` is the domain config layer; `Resolver` is the
argv-parse layer; `main()` orchestrates. Each layer holds its own
concern.

## Cross-references

- `properties.md` — property #3 (orchestrators carry no state)
- `../../std/cli.md` — `std::cli::Resolver` reference
- `../../book/01-why-aperio.md` — the orchestrator pattern as seen
  in the why-aperio essay
