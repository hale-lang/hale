# Composition

The five composition patterns. Each is how loci and types
combine at call sites without surprise machinery.

## Self-method calls

Inside a locus body, `self.method(arg)` composes methods within
the locus. No special syntax, no virtual dispatch — the
receiver is implicit because the call site is inside the locus.

```aperio
locus MorphemeL {
    params { flavor: String = "go"; }
    fn lookup(m: String) -> String { /* ... */ }
    fn suffix_rule(m: String) -> String {
        let stem = self.lookup(m);   // self-method call
        // ...
    }
}
```

## Cross-locus method calls

`other.method(arg)` works when `other` is a typed locus
reference. Methods resolve by the locus's declared name; no
dynamic dispatch beyond the declared interface.

```aperio
let resolver = std::cli::Resolver { defaults: cfg };
let value = resolver.get("flavor");   // cross-locus call
```

## Let-bound locus literals

Binding a locus literal with `let` defers its dissolve to the
enclosing function's scope exit. Use this when the locus's
lifecycle should match a function body's duration.

```aperio
fn drive(conn_fd: Int) {
    let s = std::io::tcp::Stream { conn_fd: conn_fd };
    s.write_all("hello");
}   // s.dissolve() fires here
```

## Statement-position literals

A locus literal in statement position (no `let`) fires and
dissolves at the end of the *expression*. Use this when you
want a one-shot run with no aftermath.

```aperio
fn main() {
    OnboardL { dir: "fixture", flavor: "go" };   // one-shot
}
```

## Cross-locus state via bus subjects

Cross-locus state flows through bus subjects, not through
field reads on a passed reference. The bus is the
language-blessed channel for cross-locus coordination.

```aperio
"chat.message" <- Message { sender: "alice", body: "hi", ts: now() };
```

A subscribing locus in the same binary or a different one
receives a copy in its handler. No shared mutable state across
the boundary.

## See Also

- [Pattern catalog](./patterns.md) — the six shapes these
  compositions connect.
- [Bus dispatch and routing](../bus/index.md) — the formal
  semantics of subject-based dispatch.
- [Rolling the design](./rolling.md) — composition is one of
  the two payoffs of rolling (the other is shape continuity).
