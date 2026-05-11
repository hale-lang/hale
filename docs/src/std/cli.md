# `std::cli`

Command-line interface helpers. v0 ships a single namespace
lotus — `std::cli::Resolver` — that layers a value's source
across (CLI argv → environment → built-in fallback) so an app
can hand its argv-and-env handling to one configured locus
instead of hand-rolling the precedence dance per binary.

The Resolver is half of the exemplary Aperio config ritual:
**locus** (the resolver, focus of control during resolution),
**type** (the caller-defined Config shape, the collapse
target), **locus** (the app, consuming the resolved Config).
Per the types-vs-loci axiom, the flow lives in a locus and
the result lives in a type. `main()` reads as a three-step
composition.

## Loci

### `std::cli::Resolver`

A namespace lotus parameterized by an env-variable prefix and
a newline-separated positional argv key list.

#### Synopsis

```aperio
locus std::cli::Resolver {
    params {
        env_prefix: String = "APERIO_";
        argv_keys:  String = "";
    }
    fn get(key: String, fallback: String) -> String;
    fn get_int(key: String, fallback: Int) -> Int;
}
```

#### Use

```aperio
type OnboardConfig {
    dir: String;
    flavor: String;
    max_depth: Int;
}

fn main() {
    let r = std::cli::Resolver {
        env_prefix: "APERIO_",
        argv_keys:  "dir\nflavor\nmax_depth\n",
    };
    let cfg = OnboardConfig {
        dir:       r.get("dir",        "apps/onboard/fixture"),
        flavor:    r.get("flavor",     "go"),
        max_depth: r.get_int("max_depth", 4),
    };
    OnboardL { config: cfg };
}
```

#### Precedence

`get` returns the highest-populated source for `key`. Order
high to low:

1. **CLI positional argv** — if `key` appears in `argv_keys`
   at position N and the process was launched with at least N
   positional arguments, `argv[N]` wins.
2. **Environment variable** — `<env_prefix><UPPER(key)>`.
   E.g., prefix `"APERIO_"` and key `"max_depth"` resolves
   `APERIO_MAX_DEPTH`.
3. **Fallback** — the second argument to `get`.

A layer that is unset / unfilled falls through to the next.
A layer with an empty-string value is treated as populated
(the caller can distinguish presence from absence by checking
the returned value).

#### Method semantics

- **`get(key, fallback)`** resolves a String per the
  precedence above. Returns `fallback` only when every layer
  is unpopulated.
- **`get_int(key, fallback)`** resolves a String per `get`,
  then parses to `Int`. Empty / non-parseable values fall
  through to `fallback` rather than crash — a typo'd
  `--depth foo` becomes the default instead of aborting the
  app before any lifecycle method has run.

#### Why a stringly-keyed shape

Without generics or reflection (neither shipped at v0), a
generic "populate this Config type's fields" interface would
need per-field machinery the seed can't express. The
stringly-keyed shape is the v0 approximation: std owns
precedence + env-prefix + argv-position mechanics; the
caller owns the Config type and the per-field `r.get(...)`
calls. The defaults at the call site read as documentation;
the Config struct literal is the contract.

#### Notes

- `argv_keys` is newline-separated. Blank lines are tolerated
  and don't advance the positional counter. Trailing newline
  is optional.
- The env-prefix is added verbatim — include any trailing
  underscore explicitly (`"APERIO_"`, not `"APERIO"`). The
  key portion is uppercased; the prefix is not.
- The Resolver has no birth/run/dissolve. Each `get` is a
  pure query against process-level state (argv + env).
- Tests can bypass the Resolver entirely by constructing the
  Config type with explicit values. The Resolver is for
  `main()`, not for unit tests.

## See Also

- [`std::env`](./env.md) — the underlying argv / env primitives
  the Resolver dispatches to.
- [`std::str`](./str.md) — `parse_int` / `can_parse_int` are
  what `get_int` uses for coercion.
