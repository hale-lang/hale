# Constitutions

Every application has a few sentences that make it *that* application:
- the books are written only through payments;
- one service settles an order;
- the storefront never touches money.

They are true on the day it is written. Six months of changes later, nothing but memory says they still are. A **constitution** writes those sentences down once, in the source, as claims the compiler checks on every build. It is how an application states its own dna: what it must go on being, whoever changes it next.

This needs nothing running. It is not the organism of [the next part](./parts/organism.md). A constitution is checked by `hale check` and `hale build` like any other claim, and holds whether or not anything ever runs the program.

## One law, every entrypoint

```hale
type Order { id: Int; cents: Int; }
topic Orders  { payload: Order; }
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
    params { front: Storefront = Storefront { }; pay: Payments = Payments { }; books: Books = Books { }; }
    claims { adopt Shop; }
    run() { self.front.take(Order { id: 1, cents: 1250 }); }
}

fn main() { App { }; }
```

`adopt Shop;` evaluates both sentences against this entrypoint's whole program graph. Suppose a later change wires the storefront to the books by a route that skips payments, or someone deletes `avoiding payments` to "simplify" the law. The build then stops at the sentence it broke, with the path that breaks it:

```text
main.hl:26:5: type error: claim `books_only_through_payments` violated: `storefront` reaches `books` — witness: `Storefront::take` -(publishes "Orders")-> `Payments::on_order` -(publishes "Settled")-> `Books::on_settled`
```

That is the whole difference from a comment or a test. The sentence is checked over every path the compiler can see, on every build, at no cost at run time.

## Written once, adopted by each program

A constitution lives outside any `main`, so a service split into several binaries writes it once, and each entrypoint adopts it. The usual home is a seed with no `main locus` of its own, a *policy seed*, which the entrypoints import together with the groups it names. Each adoption is evaluated in that entrypoint's own closed world: one text, one verdict per program.

A constitution grows by `extends`, and only by union. A derived constitution adds sentences and can never replace one it inherits, so weakening the law cannot be written at all. [Claims & the law](./claims.md#one-law-many-entrypoints) has the rules: flat names, diamonds, and groups every adopter must declare.

## Bound to where it runs

Where a program deploys is a fact about the deployment, not the source. So the binding of law to place lives in `hale.toml`:

```toml
[claims]
base = "Shop"                  # carried by every environment

[environments.dev]
constitution = "ShopDev"       # dev may add law: nothing reaches the real provider
entrypoints  = ["apps/front", "apps/pay"]

[environments.prod]
entrypoints  = ["apps/front", "apps/pay"]
source_only  = true
```

```sh
hale check --matrix            # every (entrypoint, environment) pair
```

An environment may add law, never drop it. An entrypoint listed in no environment is an error, not a skip. [Claims & the law](./claims.md#binding-a-constitution-to-a-deployment-target) walks through each case.

## The organism keeps its law the same way

When DNA governs an application ([The organism](./parts/organism.md)), the organization's own law is this same mechanism: `dna/org/law.hl` declares `constitution Org`, and the organization's main adopts it (`claims { adopt Org; }`). A generated project's `hale.toml` lists the application and the organization as two entrypoints in two environments, and one command checks both:

```text
$ hale check --matrix .
=== ./. @ local ===
ok: 1 file(s) typechecked
=== ./dna/org @ org ===
ok: 8 file(s) typechecked

ok: 2 (entrypoint, environment) pair(s) checked
```

The application keeps its own constitution, and the organism never rewrites it. Changing either law is a change of the `constitutional` class, which only the Board decides ([Shaping and governing it](./dna/shaping.md)).
