# Examples

Placeholder. The first example program is the trader/analyst pair
on grease UDP multicast input — the load-bearing first program
discussed in the design conversation.

It will live at:

- `examples/trellis-pair/analyst.lt`
- `examples/trellis-pair/executor.lt`
- `examples/trellis-pair/shared.lt`  (the perspective + closure types
                                       both binaries compile from)
- `examples/trellis-pair/README.md`  (what the program does, what
                                       primitives it exercises)

**Status.** Not yet written. Awaiting at least: the EBNF spec to
stabilize through one or two iterations; resolution of any
load-bearing open questions (`notes/open-questions.md`); decision
on which transport-binding form to commit to for UDP multicast.

When ready, the program serves three purposes:

1. **Validation of the design.** If the spec hangs together, the
   program reads cleanly. If not, the program surfaces what's
   wrong.
2. **Concrete exemplar.** A future reader who sees this program
   understands the language by induction.
3. **Empirical anchor.** Trellis is a real production-shaped
   system; writing it in lotus is a real anchor at the trading
   substrate.
