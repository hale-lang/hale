# Effects & contracts

Most of what a function does is invisible in its signature. `fn
price(book: OrderBook, qty: Int) -> Decimal` tells you the types and
nothing else — whether it touches the filesystem, blocks on a socket,
reads the clock, allocates, or publishes to the bus.

Hale lets you say so, and then holds you to it:

```hale,fragment
@no_syscall @deterministic
fn price(book: OrderBook, qty: Int) -> Decimal { ... }
```

That is a **contract**, not a hint. The compiler proves it over the
whole call graph — through helpers, through methods, through calls made
on a handle, into imported seeds, and into the part of the standard
library written in Hale. If anything reachable from `price` performs a
syscall, the build fails and the diagnostic names the path that gets
there.

Effect contracts are **opt-in**. Nothing here fires until you ask for
it; a program with no annotations behaves exactly as before. The one
exception is noted at the end — placement implies a contract, because
putting a handler on an `async_io` pool is already a claim that it
won't block.

> **Why the compiler can do this.** The call graph is closed (no
> function pointers into the unknown, no dynamic dispatch you can't
> resolve), the stdlib surface is classified entry by entry, and the
> message graph is declared. Those three facts are what turn "what does
> this function do?" from a code-review question into a compile-time
> one.

## Declaring what a function may do

A fn can declare what it's allowed to *reach*, and the compiler
enforces it across everything reachable from it:

```hale,fragment
@effects(none: {syscall, block}) fn decode(b: Bytes) -> Msg { ... }
@effects(none: {time})           fn backoff(n: Int) -> Int { ... }
@effects(publish: {OrderFill})   fn route(o: Order) { ... }
```

`none: {…}` forbids effect classes — `syscall`, `block`, `time`,
`entropy`, `env`, `ffi`, `publish`, `spawn`, `recursion`.
`publish: {…}` declares which topics a fn may publish to (exact,
because Hale's topic set is closed).

Two things are worth knowing up front, because both surprise people:

- **`println` is a syscall.** Writing to a stream is a `write(2)`,
  it can block, and a `@no_syscall` fn that printed would not be
  certifying what it claims. Debug printing inside a certified fn is
  a contract violation — which is the point.
- **The check follows handles.** `reader.slurp()` is a call like any
  other; the compiler resolves the receiver's type and walks into
  the method. Putting an effect behind a locus that the asserting fn
  still calls does not hide it. (Putting it behind a locus the fn
  *doesn't* call — a separate reader/writer locus it publishes to —
  is the real fix, and is what the diagnostic suggests.)

The common contracts have short names, which are exactly the same
thing spelled shorter:

| shorthand | full form |
|---|---|
| `@no_syscall` | `@effects(none: {syscall})` |
| `@no_block` | `@effects(none: {block})` |
| `@no_ffi` | `@effects(none: {ffi})` |
| `@no_publish` | `@effects(none: {publish})` |
| `@no_spawn` | `@effects(none: {spawn})` |
| `@no_recursion` | `@effects(none: {recursion})` |
| `@deterministic` | `@effects(none: {time, entropy, env})` |

They stack with each other and with the contracts above, so the full
hot-path certificate is one line:

```hale,fragment
@no_block @no_syscall @deterministic @no_recursion @hot
@budget(alloc_per_call = 0)
fn on_tick(a: Int, b: Int) -> Int { ... }
```

`@deterministic` is the one to know for replay: a fn that reads no
clock, no randomness and no environment is a function of its inputs,
so replaying its inputs replays its behavior exactly. The compiler
knows the difference between reading a source and using a supplied
value — `std::time::time_from_unix(at)` formats an instant you
passed in and is fine; `std::time::monotonic_ns()` is not.

Reach for the general form when the shorthand doesn't say what you
mean. "No wall clock, but jitter is fine" is a real contract for a
retry loop, and it's `@effects(none: {time})` — entropy stays
allowed.

When an assertion fails you get the path, not just the verdict:

```
effect assertion violated: `on_tick` must not reach `block`, but reaches
  on_tick -> helper -> nap [std::time::sleep — a blocking operation …].
```

## Counting, not just forbidding

`@budget` counts other dimensions too, comma-separated in one
clause:

```hale,fragment
@budget(alloc_per_call = 0, stack_bytes = 4096, block_points = 0)
fn on_frame(x: Int) -> Int { ... }

@budget(publish = 1) fn reply(o: Order) { ... }   // exactly-once reply
@budget(fanout = 8)  fn notify(e: Ev) { ... }     // bounded amplification
```

`stack_bytes` is the deepest call *chain*, not the sum of everything
reachable — two 100-byte helpers called side by side cost 100, not
200. Recursion makes it unbounded, which is why it pairs naturally
with `@no_recursion`.

`fanout` is the one that catches surprises: it counts transitive
subscriber *deliveries*, so publishing once to a subject with 200
subscribers is a fan-out of 200. That's the amplification a per-fn
count can't show you.

## Effects by lifecycle phase

A locus can say which effects each phase may perform:

```hale,fragment
@phase_effects(birth: {alloc}, run: {})
locus Engine { ... }
```

That single line is the "no dynamic memory after initialization"
discipline: allocate while starting up, never in the steady state.

**A phase name** is one of the six lifecycle hooks — `birth`,
`accept`, `release`, `run`, `drain`, `dissolve` — or **any handler or
method by name**, which is how you write a per-message contract:

```hale,fragment
@phase_effects(on_order: {publish, alloc})
locus Router { ... }
```

Note the `alloc` there. The set is **exact**, and building the payload
you publish is an allocation — so a handler that constructs `Ack { … }`
and sends it needs both classes. `{publish}` alone is a stricter claim
than most publishing handlers can honour, and the compiler will say so.

The lifecycle names are always legal, written out or not: a locus with
only `params` still has a birth. A phase that names *nothing* on the
locus — a typo, or a handler you renamed — is an error, because a
contract nobody checks is worse than no contract.

**Empty braces and omission are opposites**, and this is the part to
get right:

| form | means |
|---|---|
| `run: {}` | `run` may perform **no** effect at all |
| `run: {alloc}` | `run` may allocate, and do nothing else |
| *(`run` not mentioned)* | `run` is **unconstrained** |

So `@phase_effects(birth: {alloc})` alone constrains only birth —
`run` stays free. It takes the explicit `run: {}` to say "and nothing
in the steady state." Violations are errors, and the diagnostic is
prefixed with the phase: ``phase `run`: effect assertion violated…``

## Never trapping

`@no_panic` asks a different question from the effect classes — not
"what do you reach?" but "can any path here fail?":

```hale,fragment
@no_panic fn parse_frame(b: Bytes) -> Frame {
    return decode(b) or Frame { };   // handled — fine
}
```

An explicit `violate`, an `or raise` (which propagates rather than
handles), or a trapping index all violate it. `or discard`, a
substitute value, and `or handler(err)` satisfy it.

## The assertion you don't have to write

Placement already declares intent, so some contracts need no
annotation. A locus placed `cooperative(pool = web) where async_io`
shares that pool's single worker — so a handler on it that blocks
holds up every other locus on the pool:

```
`Worker::on_e` is placed on the async_io pool `web`, whose single worker
it shares — but it reaches Worker::on_e -> nap [std::time::sleep …].
A blocking call here stalls every other locus on `web` until it returns.
```

That's a warning, not an error: a locus that owns its pool may block
on purpose. Writing `@no_block` on the handler says "I mean it" and
upgrades the check to an enforced error.

## The classes

| class | covers | example |
|---|---|---|
| `syscall` | the kernel: filesystem, sockets, processes, terminal, stdio — including `println` | `std::io::fs::read_file` |
| `block` | waits, holding its thread (or, on an `async_io` pool, its worker's turn) | `std::http::get` |
| `time` | *reading* a clock | `std::time::monotonic_ns` |
| `entropy` | *reading* randomness | `std::rand::next_int` |
| `env` | *reading* the environment or argv | `std::env::var` |
| `ffi` | reaching an `@ffi` declaration — leaving managed Hale | any `@ffi` fn |
| `publish` | sending on the bus | `Orders <- o` |
| `spawn` | instantiating a locus | `Worker { }` |
| `recursion` | a cycle in the call graph | a fn that reaches itself |
| `alloc` | arena allocation — a phase-only class, see `@phase_effects` | `Buf { n: 1 }` |

Two of those are worth dwelling on, because they are the ones people
guess wrong.

**`println` is a `syscall`.** Writing to a stream is a `write(2)`; it
can block, and a certificate that permitted it would not be certifying
what it claims. Debug printing inside a `@no_syscall` function is a
contract violation — which is the point of having asked.

**Reading an effect source differs from operating on a supplied
value.** This is the distinction that makes `@deterministic` useful
rather than merely restrictive:

| pure | effectful |
|---|---|
| `std::time::time_from_unix(n)` | `std::time::monotonic_ns()` — `time` |
| `std::str::parse_int(s)` | `std::env::var(k)` — `env` |
| `std::http::parse_request(b)` | `std::http::get(u)` — `syscall`, `block` |

A function handed a timestamp is a function of its inputs. A function
that *fetches* one is not, and cannot be replayed.

You never have to work this out from memory. `hale doc --stdlib`
publishes every function's classes alongside its signature, generated
from the same registry the checker queries — so the catalogue and the
enforcement cannot disagree:

```
### std::io::fs::read_file

    fn read_file(String) -> String fallible(IoError)

**Effects:** `syscall`
```

The two sections that follow are what make a contract worth relying on
rather than decorating with: how far the proof actually reaches, and
what the compiler will tell you about code nobody annotated.

## How far the proof reaches

A violation names the **path**, not just the function:

```
type error: effect assertion violated: `quiet` must not reach `syscall`,
but reaches quiet -> Reader::slurp [std::io::fs::file_size — a
syscall-class operation (filesystem, socket, process, terminal, stdio)].
Move the effect behind a locus this fn doesn't reach (the
reader/writer-locus shape), or pass the value in as a parameter.
```

That chain crosses ordinary calls, `self` methods, and calls made
through a **handle** (`reader.slurp()`) — including into the part of the
standard library that is itself written in Hale, whose effects are
inferred from its implementation rather than declared in a table.

Read the suggestion precisely. Moving an effect behind a locus your
function *still calls* does not remove it; the checker follows the
handle. What works is a locus your function does not reach — a
reader/writer locus it publishes to — or passing the value in already
computed.

Incompleteness fails closed. A stdlib call the registry cannot classify
— or does not know at all — is treated as "may do anything" and
violates the assertion, rather than being quietly assumed pure. An
assertion is a claim about everything reachable, so anything unknown is
exactly what it must not certify.

## What the compiler infers, as a reviewable artifact

Annotations only describe the functions someone remembered to annotate.
The manifest describes **every** function:

```sh
hale check app.hl --dump-effects-manifest > .hale.effects
```

Each line carries the declared contract plus a `does={…}` column — what
the compiler sees the function actually do, transitively, annotated or
not, for free functions, locus methods and lifecycle hooks alike:

```
# .hale.effects v1 — declared effect contracts
App::run     does={syscall,block,time}
Pusher::run  does={syscall,block,publish,time,alloc}
```

Commit that file and a later build can diff against it:

```sh
hale check app.hl --check-effects-manifest .hale.effects
```

```
- Api::emit  none={block}  does={publish,alloc}
+ Api::emit  none={block}  does={syscall,publish,alloc}
```

This catches what annotations structurally cannot. `Api::emit` gained
filesystem I/O and *nothing in its source changed* — a helper three
calls away did. No contract was violated, because none was declared;
the behaviour simply drifted. As a CI gate it turns that into a
one-line review diff. (The Hale compiler runs this against its own
example corpus.)

## Effects that travel over the bus

A call graph stops at a publish. The **bus graph continues** — and
because Hale's message graph is declared over a closed topic set, the
compiler can walk it:

```hale,fragment
@effects(causes: {publish, syscall})
fn handle(o: Order) { Orders <- o; }
```

`causes:` is a **complete declaration**, like `publish:` — not a list
of things to forbid. You state everything publishing can transitively
set off, and omitting one is the error:

```
type error: declared causal set violated: `Api::handle` can transitively
cause syscall through the bus, which its `@effects(causes: …)` does not
declare. Path: `Api::handle` -> subject `Orders` -> `Audit::on_order`.
Add the class to the declaration, or route the publish to a subject
whose subscribers don't perform it.
```

Only effects reached *through* the bus count here; direct ones are
`none:`'s job. "Publishing is cheap" is often false because of what
runs downstream, and this is the only place the compiler can see it —
a system built on opaque message sends cannot ask the question.
