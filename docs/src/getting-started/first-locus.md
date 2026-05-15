# Your first locus

Save the following as `hello.ap`:

```aperio
locus Greeter {
    params { name: String = "world"; }
    birth() { println("hello, ", self.name); }
}

fn main() {
    Greeter { };
    Greeter { name: "Aperio" };
}
```

Run it interpreted:

```sh
aperio run hello.ap
```

You should see:

```
hello, world
hello, Aperio
```

## What just happened

`Greeter` is a **locus**: a typed unit with a lifecycle. `params`
declares its configurable state with defaults; `birth()` is the
lifecycle method that runs when an instance is constructed.

`Greeter { }` constructs an instance using the default `name`;
`Greeter { name: "Aperio" }` overrides it. Both instances run
their `birth()` body to completion, then dissolve at the end of
the surrounding statement.

## Adding a topic

Topics make loci communicate without referring to each other by
name. The publisher publishes; the subscriber subscribes; both
sides only mention the topic.

```aperio
type Tick { n: Int; }
topic Beats { payload: Tick; }

locus Counter {
    params { sum: Int = 0; }
    bus { subscribe Beats as on_beat; }
    fn on_beat(t: Tick) { self.sum = self.sum + t.n; }
}

locus Pulse {
    params { iters: Int = 4; }
    bus { publish Beats; }
    run() {
        let mut i = 1;
        while i <= self.iters {
            Beats <- Tick { n: i };
            i = i + 1;
        }
    }
}

fn main() {
    let c = Counter { };
    Pulse { iters: 4 };
    print("sum=");
    println(c.sum);
}
```

The output is `sum=10` (1+2+3+4). `Counter` outlives the `Pulse`
because it's `let`-bound; `Pulse` is a statement-position
construction, so it runs through `run()` and dissolves before
the `print`.

## Where to go next

- Read the example fixtures under
  `crates/aperio-codegen/tests/fixtures/examples/` for a tour of
  language features.
- Read [`spec/styleguide.md`](https://github.com/local/lotus-lang/blob/main/spec/styleguide.md)
  for idiomatic patterns.
- For deeper questions, the canonical references are
  [`spec/semantics.md`](https://github.com/local/lotus-lang/blob/main/spec/semantics.md)
  and [`spec/grammar.ebnf`](https://github.com/local/lotus-lang/blob/main/spec/grammar.ebnf).
