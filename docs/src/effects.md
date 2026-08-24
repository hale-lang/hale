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
`entropy`, `env`, `ffi`, `publish`, `spawn`, `recursion`,
`secret_use`.
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

Three details matter once you start relying on it:

- It counts **runtime registrations**, not declarations. One
  `subscribe` line on a locus arranged with `replicas = 3` is three
  deliveries.
- It is **transitive**. If a handler you reach republishes, those
  deliveries are yours too — a publish reaching one relay that
  republishes to three sinks is a fan-out of four, not one.
- One message carries **one key**. A keyed publish reaches the
  subscriptions whose filter that key satisfies, so
  `where key == replica` costs one delivery, not one per replica.

Where the compiler cannot know the number — a computed subject, a
subscriber born outside the arrangement, a topic bound to a
transport — the answer is *unbounded*, not a guess. A budget cannot
hold over a count nobody can take.

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

The closed set covers **your own effect classes** too: a phase that
reaches a declared `money` carrier without listing `money` violates,
and `run: {money}` permits it — the same closed-universe rule as
`only:`, composed classes excluded from the complement (they own no
bit of their own).

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
| `secret_use` | a privileged operation over confined secret material | `std::secret::Signer.sign` |

`secret_use` is **compiler-owned**: `std::secret`'s privileged methods
carry it, and you use it without declaring it — in `none:`, `only:`,
`@phase_effects`, `forbid reaches(G, effects(secret_use))`, and
`bound secret_use <= N`. Every built-in name is reserved, so
`effect secret_use;` (or `effect syscall;`) is an error rather than a
silent no-op. See [Verification](./verification.md#secrets-confine-classify-claim).

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

## …and effects that travel *toward* you

`causes:` walks the bus graph forward. `depends:` walks it backward —
the complete set of subjects that can transitively reach any of a
locus's handlers:

```hale,fragment
@effects(depends: {Recalled})
locus StatedCarry {
  bus { subscribe SumLookup as on_sum; }
}
```

Without it, an independence claim is unenforceable. A locus that
subscribes only to `SumLookup` looks isolated from `Recalled` in
every declaration it carries — but if some third locus subscribes to
`Recalled` and republishes onto `SumLookup`, the influence arrives
anyway, and nothing in the depending locus's source mentions it. The
diagnostic names the laundering path:

```
type error: declared dependency set violated: `StatedCarry` can
transitively depend on `Recalled` through the bus, which its
`@effects(depends: …)` does not declare. Path: subject `Recalled` ->
`Launderer` -> subject `SumLookup` -> `StatedCarry`.
```

You may name the topic (`Recalled`) or its wire subject
(`"recalled"`) — they address one endpoint, and the compiler joins
on that identity rather than on how you spelled it.

It sits on the **locus**, not a fn: dependence enters through
subscriptions, and those are declared per-locus. A fn-level
`depends:` is a parse error rather than a silent no-op.

It is opt-in, on measured grounds. Over a real application — 428
topics, 114 loci — transitivity added nothing beyond what `bus {}`
already said for 87% of loci. A mandatory form would be redundant far
more often than it was informative. Reach for it where independence
is load-bearing: a locus that must not see PII, a control plane that
must not be influenced by user traffic.

One boundary worth stating plainly: the closure is over the **bus
graph**. Influence that travels outside it — through a shared form,
through a file — is not part of it.

## Effects you declare yourself

The ten classes above are the compiler's. A program can name its own:

```hale
effect money;

@effects(is: {money})
fn charge(cents: Int) -> Bool { return cents > 0; }

@effects(none: {money})
fn quote(cents: Int) -> Int { return cents * 2; }

fn main() { println(quote(100)); }
```

`effect money;` declares the class. `is:` classifies a function as a
source of it. From there it is an ordinary effect: it propagates
through the call graph, it travels over the bus under `causes:`, it
can be forbidden with `none:`, and a violation reports the same
witness path a built-in would.

```
type error: effect assertion violated: `quote` must not reach `money`,
but reaches quote [`charge` declares it carries this effect class].
Move the effect behind a locus this fn doesn't reach (the
reader/writer-locus shape), or pass the value in as a parameter.
```

The usual objection is that a user effect has no frontier — that
`none: {money}` can only ever mean "no function anybody remembered to
annotate", which is a linting convention wearing a type system's
clothes. It's a fair objection to answer rather than wave at.

The answer is that the effects worth checking are about interaction
with the outside, and that *is* the frontier. Money moves when the
payment processor is called, when the ledger row is written, when the
settlement is published — all of which are already frontier calls the
compiler classifies. `is:` doesn't introduce a second kind of
grounding; it adds rows to the classification that already exists.
The compiler owns propagation; the program owns classification — the
same split the stdlib registry has, with a different owner.

That split is also the honest statement of what you get. If you
annotate `charge` and forget its sibling, `none: {money}` will not
catch the sibling. What it does guarantee is that *given* your
classification, no path escapes — which is exactly the property that
is tedious and error-prone to maintain by review, and exactly the one
that stops holding the moment a call graph is more than a few edges
deep.

Classes cross seed boundaries — a class declared in a library resolves
from the app that imports it, which is the whole point: `money` should
hold everywhere the money goes, and in a real codebase the money goes
through `lib/`. Each seed interns its own names from zero, so the merge
unions the tables and remaps each seed's indices before combining them.

There are 54 classes available — the bits above the built-ins in the
effect mask. Declaring past that is an error at the `effect NAME;`
line, not a saturating no-op: a class with no bit unions as "reaches
nothing", so `@effects(none: {it})` would silently certify a function
that calls a declared source. Everything else here fails closed, and
a certificate that is quietly false is worse than none, because it is
believed.

## Saying what a function may do, not what it may not

Everything so far forbids. `@effects(none: { syscall })` names what is
out of bounds and permits the rest — which is the right shape when you
care about one or two things.

It is the wrong shape when you care about *everything else*. To say
"this handler allocates and does nothing more" you would have to write
out every other class:

```hale,fragment
@effects(none: { syscall, block, publish, time, entropy, env, ffi, spawn, recursion })
```

And that list is a snapshot. Add a class to the language — or declare
one of your own — and every contract like it silently permits the new
class. The annotation still reads "only alloc"; it no longer means it.
Nothing fails. The certificate just quietly weakens, which is the one
failure mode this system exists to rule out.

So there is a closed form:

```hale
@effects(only: { alloc })
fn label(n: Int) -> String {
    return "a" + "b";
}
```

The inferred effect set must be a **subset** of what you listed. The
complement is computed when the program is checked, from the classes
that actually exist — the ten built-ins plus every class the program
declares. Nothing is written down, so nothing can go stale:

```hale
effect money;

@effects(is: { money })
fn charge(cents: Int) -> Int { return cents; }

@effects(only: { alloc })
fn quote(n: Int) -> Int { return charge(n); }   // violation

fn main() { println(quote(1)); }
```

`quote` never mentions `money`. It is caught anyway, because `money`
is not in the permitted set — and it would still be caught if `money`
were declared a year after `quote` was written. That is the whole
difference between the two forms.

## Classes built from other classes

A class can be defined as the union of others:

```hale,fragment
effect io = { syscall, block };
```

`io` has no bit of its own — its mask *is* `syscall | block`. One fact,
two useful consequences, no extra machinery: forbidding `io` catches
either member, and a function that reaches a syscall carries `io`.

This is also how a user class joins a built-in contract. `@deterministic`
means "must not read anything that varies", and it is defined over
`time`, `entropy` and `env` — so a class of your own is invisible to it:

```hale,fragment
effect wallclock;              // atomic: @deterministic cannot see it
effect wallclock = { time };   // composed: it can
```

The second form says *your class is a kind of clock read*, which puts
the `time` bit in its mask, which is exactly what `@deterministic`
already tests. Nothing about the contract changes.

It stays opt-in, and that is deliberate. An atomic class like `money`
is **not** swept into `@deterministic` — moving money is not a source
of nondeterminism, and the compiler has no business guessing that it
is. You say so by defining the class in terms of what it actually is.

A definition may name other declared classes as well as built-ins. A
cycle — `effect a = { b }; effect b = { a };` — resolves to no effect
at all, so every contract naming either would hold vacuously. Cycles
are rejected rather than left inert.
