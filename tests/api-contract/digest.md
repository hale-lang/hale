# The digest of `Public`, by hand

`spec/api.md` § The contract digest states the rule; this is the rule
applied to `program.hl`'s `Public`, step by step, so a consumer can
check its own implementation against every intermediate value.
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
| `Orders::place` | `PlaceOrder` | `OrderReceipt` | none | none |
| `Orders::cancel` | `CancelOrder` | `Cancelled` | `OrderError` | `trader` |

`Orders::cancel` takes `ctx: std::api::Context` after its request; the
`Context` is not part of the request.

## 2. Canonical order

By member name, compared as bytes: `Orders::cancel` before
`Orders::place` (`c`, 0x63, before `p`, 0x70).

## 3. Each type's shape and shape hash

The canonical structural shape: the struct's fields in declaration
order as `<field>:<tag>` joined by `;`, an identity tagged `i` as the
`Int` it is, a quantity tagged by its denomination (`q(cent)`). Its
hash is the 64-bit FNV-1a fold of the shape's bytes (offset basis
`0xcbf29ce484222325`, prime `0x100000001b3`), as sixteen lowercase hex
digits. The shapes are the compiler's: `hale check --dump-topology` on
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
```

## 4. The hash input

The header line, then one line per row in canonical order: member,
request, response, error, requires, separated by one TAB (0x09); `-`
where the row has none; every line ended by one LF (0x0A). Shown with
each TAB written `→`:

```text
hale-api-surface 1
Orders::cancel→deb8489f34994e5a→e1506381a35c8ced→db0311924c0e7333→trader
Orders::place→cb5775974312c858→bb4f99639cf069af→-→-
```

The exact bytes, 144 of them, in hex:

<!-- input: Public -->
```text
68616c652d6170692d7375726661636520310a4f72646572733a3a63616e6365
6c09646562383438396633343939346535610965313530363338316133356338
6365640964623033313139323463306537333333097472616465720a4f726465
72733a3a706c6163650963623537373539373433313263383538096262346639
3936333963663036396166092d092d0a
```

## 5. The digest

The 64-bit FNV-1a fold of those 144 bytes:

```text
fnv1a64:fe65e6d3036ee1eb
```

The exposures `public` and `partner` serve this surface, so both carry
it: `Public@fnv1a64:fe65e6d3036ee1eb/public` and
`Public@fnv1a64:fe65e6d3036ee1eb/partner`. Neither the listener, the
receiver instance (`self.orders`, `self.partner_orders`), the role
source nor the surface's name entered the input.

## `Admin`, for a second check

```text
hale-api-surface 1
Ledger::rebalance→19611780fbd68ecf→3193bf68569ed280→-→operator
Orders::cancel→deb8489f34994e5a→e1506381a35c8ced→db0311924c0e7333→operator
```

<!-- input: Admin -->
```text
68616c652d6170692d7375726661636520310a4c65646765723a3a726562616c
616e636509313936313137383066626436386563660933313933626636383536
396564323830092d096f70657261746f720a4f72646572733a3a63616e63656c
0964656238343839663334393934653561096531353036333831613335633863
65640964623033313139323463306537333333096f70657261746f720a
```

157 bytes, folding to `fnv1a64:98daa4b3e265ed98`.
`Orders::cancel`'s line differs from `Public`'s only in its required
role: the same handler under another surface's `requires` is another
contract.
