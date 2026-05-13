# Pattern catalog

Six patterns. If a piece of Aperio code doesn't match one of
these, look again at how the language wants you to express it.

## 1. App locus — outer encapsulation

Every app's `main.ap` defines an `<Name>L` locus that owns the
whole run. `fn main()` reads argv, instantiates the locus, exits.
The locus's `run()` body delegates to a free helper when
short-circuit logic is needed (lifecycle bodies reject
`return`).

```aperio
locus OnboardL {
    params {
        dir: String = "apps/example/fixture";
        flavor: String = "go";
    }
    run() {
        __drive(self.dir, self.flavor);
    }
}

fn main() {
    let mut dir = "apps/example/fixture";
    let mut flavor = "go";
    if std::env::args_count() > 1 {
        dir = std::env::arg(1);
    }
    if std::env::args_count() > 2 {
        flavor = std::env::arg(2);
    }
    OnboardL { dir: dir, flavor: flavor };
}
```

Conventions:

- Locus name is `<FileStem>L` with `L` suffix.
- `params` block holds argv-derived configuration with
  reasonable defaults (so the app self-demos with no flags).
- `run()` is the only lifecycle method most apps need; drop
  the `fn` keyword for lifecycle methods.
- `main()` does the argv parsing, then a single statement-
  position locus literal kicks the run.
- Statement-position literals fire-and-forget: `OnboardL { ... };`
  starts the run and the locus dissolves at function return.

## 2. Namespace lotus — empty params, only methods

When a coherent set of pure helpers forms a vocabulary, wrap
them in a locus with `params { }` and a method-only body. Use
sites instantiate once and dispatch through it.

```aperio
locus MorphemeL {
    params {
        flavor: String = "go";
        overrides: String = "";
    }
    fn lookup_morpheme(m: String) -> String {
        // ...
    }
    fn suffix_rule(m: String) -> String {
        if self.ends_with(m, "er") {
            // ...
        }
    }
    fn name_to_motion(name: String) -> String {
        let hit = self.lookup_morpheme(name);
        // ...
    }
}

// Use site:
let r = std::lang::Morpheme { flavor: "go" };
let motion = r.name_to_motion("OrderProcessor");
```

Notes:

- "Empty params" doesn't have to be literally empty —
  `MorphemeL` carries `flavor` and `overrides` because they
  parameterize the lookup. The point is no lifecycle *state*
  that birth/run/dissolve would mutate.
- One alloc per instantiation. Negligible.
- Self-method calls (`self.X(...)`) work and are how methods
  compose internally.

## 3. Service locus — long-lived with lifecycle + bus

When the thing genuinely runs over time and participates in
the bus, write the full lifecycle.

```aperio
locus TcpListenerL {
    params {
        host: String = "127.0.0.1";
        port: Int = 0;
        listen_fd: Int = -1;
        max_accepts: Int = 1;
        on_connection: fn(std::io::tcp::Stream) = __default_on_connection;
    }
    birth() {
        self.listen_fd = std::io::tcp::__listen_socket(self.host, self.port);
    }
    run() {
        let mut accepted = 0;
        while self.max_accepts < 0 || accepted < self.max_accepts {
            let conn = std::io::tcp::__accept_one(self.listen_fd);
            __handle_one_connection(conn, self.on_connection);
            accepted = accepted + 1;
        }
    }
    dissolve() {
        std::io::tcp::__close_fd(self.listen_fd);
    }
}
```

Conventions:

- `birth()` does setup that needs to run before any work.
  Mutate `self.field` to record acquired resources.
- `run()` does the long-lived work. Often a loop that consumes
  some bounded budget or runs forever depending on
  configuration.
- `dissolve()` releases what `birth()` acquired. Both are
  required if either is.
- Sentinel values in `params` (`-1` for "not yet bound") let
  `dissolve()` safely no-op on partially-constructed loci.

## 4. Spawned child locus — let-bound, scope-dissolves

When a parent locus's `run()` produces work that needs its own
lifecycle (a per-connection handler, a per-task worker),
let-bind a locus literal. Deferred-dissolve fires the child's
`dissolve()` at the parent function's scope exit.

```aperio
fn __handle_one_connection(conn_fd: Int, on_conn: fn(std::io::tcp::Stream)) {
    let s = std::io::tcp::Stream { conn_fd: conn_fd };
    on_conn(s);
}
```

The `let s = ...` binds the `Stream` locus to the function's
scope; when `__handle_one_connection` returns, `s.dissolve()`
fires (which closes `conn_fd`). No explicit `dissolve(s)` call
needed.

Conventions:

- Use let-binding when the locus needs to live for a function
  body's full duration. Statement-position literals fire and
  dissolve at the end of the *expression*, which is rarely
  what you want.
- Per-iteration cleanup uses a free helper whose return is
  the per-iteration boundary (as `__handle_one_connection`
  does for the listener loop). Block-level deferred-dissolve
  isn't shipped at v0; this is the workaround.

## 5. Shape type — pure data, no flow

When a thing IS data, not flow, declare it as `type`. No
lifecycle, no contracts, no bus.

```aperio
type HttpRequest {
    method: String;
    path: String;
    version: String;
    body: String;
}

type HttpResponse {
    status: Int;
    content_type: String;
    body: String;
}
```

Use sites construct via struct literal:

```aperio
let req = std::http::Request {
    method: "GET",
    path: "/",
    version: "HTTP/1.1",
    body: "",
};
```

Conventions:

- PascalCase, no `L` suffix (the suffix is reserved for loci).
- All fields named in snake_case.
- Returnable from functions by value. No lifecycle implications.
- If you find yourself wanting to add methods to the type,
  you've discovered it's actually a locus — change the keyword.

## 6. Free fn — sparingly

Free `fn`s at the top of a file are appropriate in three cases:

1. **`return`-bearing helpers** called from lifecycle method
   bodies (which cannot themselves use `return` at v0). See
   the App locus pattern's `__drive`.
2. **Extension hooks** passed via fn-pointer params (e.g.,
   `on_connection: fn(Stream)`). The hook is named at the top
   level so a caller can pass it by name.
3. **Genuinely-isolated helpers** that don't fit any
   namespace. Rare. If three such helpers accumulate, they
   probably form a namespace lotus you haven't extracted yet.

Free fns are NOT exported across seed boundaries. Anything you
want another seed to call should live as a method on a locus.

## See Also

- [Naming](./naming.md) — the conventions for each construct
  above.
- [Composition](./composition.md) — how the patterns compose
  at call sites.
- [Rolling the design](./rolling.md) — the rule for adding a
  seventh shape (don't, unless rolling).
- [Anti-patterns](./anti-patterns.md) — what reaching for a
  primitive from another language looks like.
