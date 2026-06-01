# Your first program

> Everything from this level, in one small CLI.

Let's build a complete little command-line tool using only what
*the basics* covered: variables, math, functions, control flow,
and the fallible model. It converts a Celsius temperature passed
on the command line into Fahrenheit.

```hale
fn c_to_f(c: Float) -> Float {
    return c * 9.0 / 5.0 + 32.0;
}

fn main() {
    // arg(0) is the program name; arg(1) is the first real argument.
    let raw = std::env::arg_or(1, "20");

    let celsius = std::str::parse_float(raw) or {
        eprintln("not a number: ", raw);
        return;
    };

    let f = c_to_f(celsius);
    println(raw, "C = ", to_string(f), "F");
}
```

Run it:

```sh
hale run temp.hl 100
```

```
100C = 212F
```

With no argument it falls back to `"20"` and prints `20C =
68F` — the tool self-demonstrates.

## What each piece is doing

- **`std::env::arg_or(1, "20")`** reads command-line argument 1,
  or `"20"` if there isn't one. (`std::env::args_count()` and
  `std::env::arg(i)` are the lower-level pair.)
- **`std::str::parse_float(raw) or { ... }`** addresses the
  fallible parse. Here the `or` arm prints to standard error and
  returns early — a fine motion when the success type is a value
  but you'd rather bail than substitute. (`eprintln` is `println`
  for stderr.)
- **`c_to_f`** is a plain free function — a calculation with no
  state, exactly what free functions are for.
- **`println(raw, "C = ", to_string(f), "F")`** concatenates its
  arguments. No format string.

## This is a real program

You can `hale build temp.hl` and ship the resulting binary. It
reads input, validates it, computes, and reports — and it's
honest about failure, because the parse *had* to be addressed.
At this level Hale is a small, sharp scripting language.

You may have noticed there's no `locus` here, no `bus`, none of
the structural machinery from the introduction's matchmaker. You
don't need it yet. A program that's a handful of functions and a
`main` is a perfectly good Hale program.

The next level is where structure starts to pay off — when your
program grows state that lives over time, talks to the
filesystem and the network, and wants to be organized into named
parts. That's where the **locus** earns its place.

Next: [The locus, gently](../everyday/locus-gently.md).
