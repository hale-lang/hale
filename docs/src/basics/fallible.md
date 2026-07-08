# When a call can fail

> Hale's value-level error model — and why you can't ignore it.

Some calls can't always succeed. Parsing `"banana"` as an
integer, reading a file that isn't there, connecting to a host
that's down. In Hale these calls have a type that says so, and
the compiler *requires* you to deal with the failure right at the
call site. There are no exceptions, no surprise control flow, and
no silently-ignored error codes.

## The `fallible` type

A function that can fail declares it with `fallible(E)`, where
`E` is the type of the error payload:

```hale
type ParseError { kind: String; input: String; }

fn parse_count(s: String) -> Int fallible(ParseError) {
    if !std::str::can_parse_int(s) {
        fail ParseError { kind: "not_int", input: s };
    }
    return std::str::parse_int(s) or 0;
}
```

`fail <payload>;` exits the function through the error path,
carrying the payload. The function's result is now "either an
`Int`, or a `ParseError`" — and the caller can't just use it as
an `Int`:

```hale
let n = parse_count(input);     // ERROR: error not addressed
```

You have to *address* the error. You do that with an `or`
clause.

## The five `or` motions

```hale
let a = parse_count(s) or raise;              // propagate upward
let b = parse_count(s) or 0;                  // substitute a value
let c = parse_count(s) or handle(err);        // hand off to a helper
let d = parse_count(s) or fail OtherErr { };  // translate the error
some_unit_call()       or discard;            // ignore (unit result only)
```

- **`or raise`** — pass the error up to *your* caller. Your
  function must itself be `fallible(E)` with a compatible error
  type, so the error has somewhere to go.
- **`or <expression>`** — substitute a fallback value of the
  success type. Inside the expression, `err` is bound to the
  payload, so you can inspect it:
  ```hale
  let port = std::str::parse_int(arg) or 8080;
  ```
- **`or handler(err)`** — call a function that takes the error
  and returns the success type. Good when several call sites
  share one recovery policy.
- **`or fail <payload>`** — fail with a *new* error of your own
  type, instead of forwarding the inner one. Use it so a library
  doesn't leak a stdlib error type through its own surface.
- **`or discard`** — throw the error away. Only allowed when the
  successful result is `()` (nothing to substitute). The compiler
  rejects `or discard` on a value-bearing call and suggests
  `or <fallback>` instead.

## A real example

Reading a file is fallible — the file might not exist:

```hale
fn load_greeting() -> String {
    return std::io::fs::read_file("welcome.txt") or "(no welcome)";
}
```

If the read fails, we substitute a default. If instead we wanted
the failure to stop us, we'd make `load_greeting` fallible and
`or raise`:

```hale
fn load_greeting() -> String fallible(...) {
    return std::io::fs::read_file("welcome.txt") or raise;
}
```

## Chaining

`or` clauses chain right-to-left — each one disposes of one
failure:

```hale
let id = parse_count(primary) or parse_count(fallback) or 0;
```

"Try the primary; if that fails, try the fallback; if *that*
fails, use 0."

## Why it works this way

This is the only failure channel you need at the basics level,
and it has a single rule: **every fallible call is addressed at
the immediate call site.** That means when you read a function
body, every place that can fail is visibly marked with `or`. No
error propagates invisibly through three stack frames; no `try`
wraps a whole block in ambiguity.

There's a *second* failure channel for a different situation — a
long-running component whose internal invariant breaks, where the
right response is a supervisor's policy rather than a return
value. That's the structural channel, and it belongs to the
services tier ([When things fail](../services/failure.md)). For
everything you'll write at this level, `fallible` + `or` is the
whole story.

Next, we put the pieces together: [Your first
program](./first-program.md).

## When the handler can fail too

A recovery handler is often itself a fallible operation — read a
fallback file, query a secondary source. You can
write that directly:

```hale
fn load(primary: String, backup: String) -> String fallible(IoError) {
    return std::io::fs::read_file(primary)
        or (std::io::fs::read_file(backup) or raise);
}
```

If the backup read succeeds, its value substitutes. If it *also*
fails, `or raise` routes the error out through YOUR function's
error path — which is why `load` must itself be `fallible` with a
compatible error type.

For your own fallible functions the inner `or raise` is implicit —
`db_read(k) or self.rebuild(k)` propagates the handler's failure
automatically. Stdlib calls and `@form` methods used as handlers
still need the explicit nested spelling above (the compiler will
tell you, with the exact rewrite, if you forget).
