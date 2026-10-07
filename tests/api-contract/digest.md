# The digests, by hand

`spec/api.md` § The contract digest states the rule; this is the rule
applied to `program.hl`'s `Public`, step by step, so a consumer can
check its own implementation against every intermediate value, then to
`Admin`, and last the stream digest of the hub `fills` (`spec/api.md`
§ Streams, the hub exposure), which folds the same way over another
framing.
`crates/hale-cli/tests/api_contract_fixtures.rs` reads the blocks
below and holds them to the fixtures: the shape hashes are the folds of
the shapes, the hash input's rows are the inventory's, and the input
folds to the digest every document of the surface carries.

**Provisional.** A shape hash here is the payload contract's (the fold
of the canonical structural shape), which R1 replaces with the
contract shape for nested and enum types (`spec/api.md`, the open
point). `program.hl`'s types are flat structs, which the payload
contract renders whole, but the values below are re-derived when R1
states the contract shape; the framing is not.

## 1. The rows

```hale,fragment
api Public {
    rpc Orders::place;
    rpc Orders::cancel requires: [trader];
}
```

| member | request | response | error | requires |
|---|---|---|---|---|
| `Orders::place` | `PlaceOrder` | `OrderReceipt` | `ClosureViolation` | none |
| `Orders::cancel` | `CancelOrder` | `Cancelled` | `OrderError` | `trader` |

`Orders::cancel` takes `ctx: std::api::Context` after its request; the
`Context` is not part of the request. `Orders::place` may violate and
returns a value, so it is `fallible(ClosureViolation)` (F.42): its error
column is `ClosureViolation`, its failure the server error, and its
error slot below is `ClosureViolation`'s shape hash, as any error
type's is.

## 2. Canonical order

By member name, compared as bytes: `Orders::cancel` before
`Orders::place` (`c`, 0x63, before `p`, 0x70).

## 3. Each type's shape and shape hash

The canonical structural shape: the struct's fields in declaration
order as `<field>:<tag>` joined by `;`, an identity tagged `i` as the
`Int` it is, a quantity tagged by its denomination (`q(cent)`). Its
hash is the 64-bit FNV-1a fold of the shape's bytes (offset basis
`0xcbf29ce484222325`, prime `0x100000001b3`), as sixteen lowercase hex
digits. The shapes are the compiler's: `hale check --dump-model` on
a program publishing each type prints them in its `topics` section.

<!-- shapes: type, shape, shape hash; one per line, TAB-free -->
```text
CancelOrder   order:i                         deb8489f34994e5a
Cancelled     order:i;was_open:b              e1506381a35c8ced
OrderError    code:s;reason:s                 db0311924c0e7333
PlaceOrder    symbol:s;qty:i;limit:q(cent)    cb5775974312c858
OrderReceipt  order:i;notional:q(cent)        bb4f99639cf069af
Rebalance     book:s                          19611780fbd68ecf
Rebalanced    moved:q(cent)                   3193bf68569ed280
ClosureViolation  locus:s;closure:s;diff:i    36c7f0561125943e
Fill          order:i;qty:i;price:q(cent)     32e4848051d36e16
```

`ClosureViolation` is the builtin record a violation carries
(`spec/semantics.md` § Inline closure violation: `locus` and `closure`
Strings, `diff` an Int, in that order).

## 4. The hash input

The header line, then one line per row in canonical order: member,
request, response, error, requires, separated by one TAB (0x09); `-`
where the row has none; every line ended by one LF (0x0A). Shown with
each TAB written `→`:

```text
hale-api-surface 1
Orders::cancel→deb8489f34994e5a→e1506381a35c8ced→db0311924c0e7333→trader
Orders::place→cb5775974312c858→bb4f99639cf069af→36c7f0561125943e→-
```

The exact bytes, 159 of them, in hex:

<!-- input: Public -->
```text
68616c652d6170692d7375726661636520310a4f72646572733a3a63616e6365
6c09646562383438396633343939346535610965313530363338316133356338
6365640964623033313139323463306537333333097472616465720a4f726465
72733a3a706c6163650963623537373539373433313263383538096262346639
39363339636630363961660933366337663035363131323539343365092d0a
```

## 5. The digest

The 64-bit FNV-1a fold of those 159 bytes:

```text
fnv1a64:a8930d6e7998e986
```

The exposures `public` and `partner` serve this surface, so both carry
it: `Public@fnv1a64:a8930d6e7998e986/public` and
`Public@fnv1a64:a8930d6e7998e986/partner`. Neither the listener, the
receiver instance (`self.orders`, `self.partner_orders`), the role
source nor the surface's name entered the input.

## `Admin`, for a second check

```text
hale-api-surface 1
Ledger::rebalance→19611780fbd68ecf→3193bf68569ed280→36c7f0561125943e→operator
Orders::cancel→deb8489f34994e5a→e1506381a35c8ced→db0311924c0e7333→operator
```

<!-- input: Admin -->
```text
68616c652d6170692d7375726661636520310a4c65646765723a3a726562616c
616e636509313936313137383066626436386563660933313933626636383536
3965643238300933366337663035363131323539343365096f70657261746f72
0a4f72646572733a3a63616e63656c0964656238343839663334393934653561
0965313530363338316133356338636564096462303331313932346330653733
3333096f70657261746f720a
```

172 bytes, folding to `fnv1a64:40381db6685c9f75`.
`Orders::cancel`'s line differs from `Public`'s only in its required
role: the same handler under another surface's `requires` is another
contract.

## The hub `fills`: its stream digest

The hub serves no surface, so its exposure is its stream rows alone:
`hub@<stream digest>/fills`. Its rows are the topic bindings to
`self.hub`, one here:

```hale,fragment
bindings {
    Fills: self.hub requires: [operator], bound: 64, on_full: drop_old;
}
```

| topic | payload | direction | codec | bound | on_full | replay | requires |
|---|---|---|---|---|---|---|---|
| `Fills` | `Fill` | `out` | `json` | 64 | `drop_old` | no | `operator` |

The header line `hale-api-hub 1`, then one line per stream row sorted
by topic name as bytes: topic, payload shape hash, direction, codec,
bound in decimal, `on_full`, replay as `0` or `1`, requires (sorted,
joined by `,`, or `-`), separated by one TAB; every line ended by one
LF. With each TAB written `→`:

```text
hale-api-hub 1
Fills→32e4848051d36e16→out→json→64→drop_old→0→operator
```

<!-- input: hub fills -->
```text
68616c652d6170692d68756220310a46696c6c73093332653438343830353164
3336653136096f7574096a736f6e0936340964726f705f6f6c640930096f7065
7261746f720a
```

70 bytes, folding to `fnv1a64:26970854397ab154`, so the exposure is
`hub@fnv1a64:26970854397ab154/fills`. Neither the listener, the hub's
name, its sources nor the subscribers entered the input; the codec,
the bound, the shedding policy and replay did, since they are what a
subscriber's loss statement is made of.

## Declaring a violation moves the digest

Before `Orders::place` and `Ledger::rebalance` were declared
`fallible(ClosureViolation)`, their error slots were `-`, and the two
surfaces folded to `fnv1a64:fe65e6d3036ee1eb` (`Public`, 144 bytes) and
`fnv1a64:98daa4b3e265ed98` (`Admin`, 157 bytes). Whether a member may
fail structurally is a fact about its contract, so declaring a violation,
or removing one, is another digest, as adding a member is.
