# Examples

A ladder of Aperio programs from the smallest viable shape
(`hello-world/`) up to a multi-binary capstone (`trellis-pair/`).
Each example has a `main.ap` (or, for multi-binary projects,
named entry points) and a `README.md` walk-through.

The ladder is also a tutorial. Read in order, each rung
introduces one new substrate primitive on top of the previous.

## Structure

- `hello-world/` — one locus, one lifecycle method, one
  built-in call.
- `01-` through `50-` — the layered tutorial: lifecycle, types,
  contracts, bus, closures, scheduling, recovery, accumulators,
  generics.
- `trellis-demo/` — single-process orchestration of the
  feedback-loop pattern (fitter + applier in one binary).
- `trellis-pair/` — the production-shaped multi-binary form:
  separate fitter and applier processes communicating over a
  typed bus.

## How to read

```bash
aperio run examples/hello-world/main.ap
aperio run examples/02-parent-child/main.ap
# ...
```

Each example's README explains what the example exercises and
the design framings it locks in.
