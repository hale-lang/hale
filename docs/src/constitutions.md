# Constitutions

A **constitution** is a reusable, named collection of architectural [claims](./claims.md) declared at the top level of a file. Constitutions allow multi-binary services and multi-environment deployments to define core architectural invariants once and enforce them across every `main locus` entrypoint at compile time via `adopt`.

```hale
type Order { id: Int; cents: Int; }
topic Orders { payload: Order; }
topic Settled { payload: Order; }

locus Storefront {
    params { taken: Int = 0; }
    bus { publish Orders; }
    fn take(o: Order) { self.taken = self.taken + 1; Orders <- o; }
}
locus Payments {
    params { settled: Int = 0; }
    bus { subscribe Orders as on_order; publish Settled; }
    fn on_order(o: Order) { self.settled = self.settled + 1; Settled <- o; }
}
locus Books {
    params { total: Int = 0; }
    bus { subscribe Settled as on_settled; }
    fn on_settled(o: Order) { self.total = self.total + o.cents; }
}

group storefront = { Storefront };
group payments = { Payments };
group books = { Books };

constitution Shop {
    books_only_through_payments: forbid reaches(storefront, books) avoiding payments;
    one_settler: count publishers(topic Settled) == 1;
}

main locus App {
    params {
        front: Storefront = Storefront { };
        pay: Payments = Payments { };
        books: Books = Books { };
    }
    claims { adopt Shop; }
    run() { self.front.take(Order { id: 1, cents: 1250 }); }
}

fn main() { App { }; }
```

When `App` includes `adopt Shop;` in its `claims { }` block, `hale check` and `hale build` evaluate every sentence in `Shop` against `App`'s closed program graph with zero runtime overhead. If a change introduces a path from `Storefront` to `Books` that bypasses `Payments` (or removes `avoiding payments`), compilation fails with the violating witness path:

```text
main.hl:26:5: type error: claim `books_only_through_payments` violated: `storefront` reaches `books` — witness: `Storefront::take` -(publishes "Orders")-> `Payments::on_order` -(publishes "Settled")-> `Books::on_settled`
```

---

## Summary of Constitution Mechanisms

| Mechanism | Syntax / Command | Purpose |
|---|---|---|
| **Declare a Constitution** | `constitution Name { <claim_name>: <form>; }` | Defines a named set of claims outside any `main locus`. |
| **Extend a Constitution** | `constitution Derived extends Base1, Base2 { ... }` | Forms the union of inherited claims and new claims (overrides are forbidden). |
| **Adopt in Source** | `claims { adopt Name; }` | Evaluates `Name`'s claims inside the enclosing `main locus`. |
| **Adopt via CLI** | `hale check <dir> --adopt <Name>` | Injects a constitution (such as `std::law::Standard`) for a single check run. |
| **Environment Matrix** | `hale.toml` `[claims]` + `[environments.<env>]`<br>`hale check --matrix` / `hale check --env <env>` | Binds base and environment-specific constitutions to entrypoints and verifies every `(entrypoint, environment)` pair. |

---

## Sharing Constitutions Across Entrypoints

### Policy Seeds and Group Vocabulary

In a multi-binary repository, place shared constitutions and `group` declarations in a **policy seed** (a library directory with no `main locus`). Each binary entrypoint imports the policy seed and writes `adopt <Name>;` inside its `main locus`.

When a constitution is adopted, its claims are evaluated against the adopting entrypoint's own closed world:

- **Declared Groups**: Every group named in an adopted constitution must exist in the adopting entrypoint (either imported from the policy seed or declared locally). If an entrypoint intentionally omits a subsystem referenced by a constitution, declare an explicit empty group with `may_be_empty`:

```hale,fragment
group payment_provider = { } may_be_empty;
```

- **Built-in `program` Group**: A constitution can reference the built-in `program` group without requiring adopters to declare it. In each adopting entrypoint, `program` resolves to all loci and free functions declared in that entrypoint's own seed.

### Additive Composition (`extends`)

A constitution can inherit from one or more base constitutions using `extends`:

```hale,fragment
constitution ShopDev extends Shop {
    no_live_gateway: forbid reaches(program, live_payment_gateway);
}
```

Constitution inheritance follows strict **union-only semantics**:

1. **No Overrides or Weakening**: A derived constitution can only add new claims. Redeclaring an inherited claim name in a derived constitution (or in the adopting `claims { }` block) is a compile error, making it impossible for a sub-constitution to silently weaken a base invariant.
2. **Diamond Deduplication**: If constitution `D` extends both `B` and `C`, and both `B` and `C` extend `A`, `A`'s claims are included once (deduplicated by declaration origin).

See [Claims & the Law](./claims.md#one-law-many-entrypoints) for complete resolution rules.

---

## Standard Library Constitutions (`std::law::Standard` & `--adopt`)

The standard library provides `std::law::Standard`, which requires all terminal output (`stdio`) in the program to pass through `std::log`:

```hale,fragment
constitution Standard {
    logs_structured: forbid reaches(program, effects(stdio)) avoiding std::log::*;
}
```

You can adopt `std::law::Standard` in source (`claims { adopt std::law::Standard; }`) or enforce it from the command line without editing source files using `--adopt`:

```sh
hale check apps/front --adopt std::law::Standard
```

`--adopt` can be passed multiple times and merges with any constitutions already adopted in source or `hale.toml`.

---

## Multi-Environment Verification (`hale.toml` & `--matrix`)

To enforce a base constitution across all deployments while adding environment-specific guardrails (for example, forbidding calls to production payment providers in `dev`), configure `[claims]` and `[environments.<env>]` in `hale.toml`:

```toml
[claims]
base = "Shop"                  # Enforced in every environment (or set `no_base = true`)

[environments.dev]
constitution = "ShopDev"       # Adds dev-specific claims on top of `Shop`
entrypoints  = ["apps/front", "apps/pay"]

[environments.prod]
entrypoints  = ["apps/front", "apps/pay"]
source_only  = true            # Enforces `base` + source claims with no extra environment constitution
```

Verify a single entrypoint under a target environment or check the entire deployment matrix at once:

```sh
hale check apps/front --env dev    # Checks apps/front with Shop + ShopDev
hale check --matrix                # Checks every (entrypoint, environment) pair
```

Key matrix rules ([Claims & the Law](./claims.md#binding-a-constitution-to-a-deployment-target)):

- Every environment automatically includes `[claims].base` (unless `no_base = true` is explicitly set), ensuring no environment can drop base invariants.
- Every environment must specify either `constitution = "<Name>"` or `source_only = true`.
- **Unlisted Entrypoints Fail**: Any seed in the workspace that declares a top-level `main locus` must be listed in at least one environment's `entrypoints` array, or `hale check --matrix` fails with an error.

---

## Constitutions in DNA-Governed Projects

When a project is governed by DNA ([The Organism](./parts/organism.md)), the organization's own governance rules use the exact same constitution mechanism:

- `dna/org/law.hl` declares `constitution Org`, which the organization entrypoint (`dna/org`) adopts via `claims { adopt Org; }`.
- The workspace `hale.toml` registers both the application (`./.`) and the organization (`./dna/org`) in its environment matrix, allowing a single command to verify both:

```text
$ hale check --matrix .
=== ./. @ local ===
ok: 1 file(s) typechecked
=== ./dna/org @ org ===
ok: 8 file(s) typechecked

ok: 2 (entrypoint, environment) pair(s) checked
```

The application maintains its own constitution independently of `Org`. Modifying either constitution is classified as a `constitutional` change, which requires Board approval ([Shaping and Governing It](./dna/shaping.md)).
