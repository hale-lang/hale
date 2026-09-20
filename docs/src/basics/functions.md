# Functions

> Naming a piece of work so you can call it.

A function is declared with `fn`, a name, typed parameters, and
an optional return type:

```hale
fn add(a: Int, b: Int) -> Int {
    return a + b;
}

fn greet(name: String) {
    println("hello, ", name);
}
```

`add` returns an `Int`. `greet` has no `-> T`, so it returns
nothing (the unit type, written `()`). Parameters are always
typed; there's no inference at the boundary, because the
signature is the contract.

Call them the obvious way:

```hale
fn main() {
    let sum = add(2, 3);          // 5
    greet("world");
}
```

## Returning a value

`return expr;` hands a value back. A function can also return its
last expression without `return` if you leave off the trailing
`;` — the block's final expression *is* its value:

```hale
fn double(n: Int) -> Int {
    n * 2          // no semicolon — this is the return value
}
```

Both styles are fine. Use whichever reads better; `return` is
clearer for early exits.

## Default parameter values

A parameter can carry a default, so the caller can leave off the
trailing arguments:

```hale
fn pow(base: Int, exp: Int = 2) -> Int {
    let mut acc = 1;
    for _ in 0..exp { acc = acc * base; }
    return acc;
}

fn main() {
    println(pow(3));      // exp defaults to 2 → 9
    println(pow(2, 5));   // override → 32
}
```

Two rules keep the calling convention unambiguous:

- **Defaults form a trailing suffix.** A required parameter can't
  follow a defaulted one — otherwise it wouldn't be clear which
  slot an omitted argument fills.
- **Defaults are evaluated at the call site**, in the caller's
  scope — not baked in when the function is defined. For a constant
  literal (the common case) that's identical; for an expression
  that names a caller-visible binding, it sees *that* binding.

Locus methods support defaults too. One caveat: bus-handler
methods and mode methods reject them — their argument shape is
fixed by the runtime, so there's no slot to fill at dispatch time.

## Functions are values

A function has a type — `fn(Int, Int) -> Int` — and you can pass
one as an argument. This is how you hand behavior to another
function:

```hale
fn apply_twice(f: fn(Int) -> Int, x: Int) -> Int {
    return f(f(x));
}

fn inc(n: Int) -> Int { return n + 1; }

fn main() {
    println(apply_twice(inc, 10));    // 12
}
```

One limit worth knowing now: a function value is just a pointer
to a named function. Hale has no *closures* — no inline
`|x| x + captured` that captures surrounding variables. If a
callback needs context, you pass the context in explicitly, or
(at higher levels) you reach for a locus that holds the state.
This keeps every function value a plain, inspectable thing.

## Free functions and where they live

A function declared at the top level of a file is a *free
function*. Every top-level declaration in a directory is visible
to every file in that directory — there's no `import` between
files in the same project, and no `pub` to mark something
exported. You organize by concern, putting related declarations
near each other, not by visibility.

```hale
// these two can call each other freely, in either file order
fn celsius_to_f(c: Float) -> Float { return c * 9.0 / 5.0 + 32.0; }
fn f_to_celsius(f: Float) -> Float { return (f - 32.0) * 5.0 / 9.0; }
```

Free functions are the right tool when an operation has no state
of its own — a calculation, a conversion, a parser. When a group
of them starts to feel like a coherent vocabulary, the
*[Everyday programs](../everyday/locus-gently.md)* level shows
how to gather them onto a locus. For now: a free function per
piece of work.

## Calling a name nothing declares

`hale check <directory>` holds a call to the same standard as a
read: the callee has to name something. A misspelled call is an
error at the call, not a mystery from the backend later:

```text
main.hl:12:14: type error: call to `celcius_to_f`: no free fn,
generic fn or fn-pointer binding with that name is in scope —
did you mean `celsius_to_f`?
```

"Something" means a free function, a generic function, a
fn-pointer binding — or one of the handful of *builtins* the
compiler answers itself, which you can call without declaring
anything: `len`, `to_string`, the two numeric casts `Int` /
`Float`, the printers (`print`, `println`, `eprint`, `eprintln`),
`abs` / `min` / `max`, `starts_with` / `contains`, and the
`bounded` collection intrinsics. Those are not magic names to
memorise — you'll meet each one where it's useful — but they are
why `len(s)` needs no import.

The list is short on purpose, and it is exact: a name that is not
on it and not declared is refused here, at the call, rather than
somewhere in the backend. A capitalised name that *looks* like a
conversion is not one — `String(x)` is not a cast, it is a call to
nothing. Use `to_string(x)` to render a value and
`std::str::parse_int` / `parse_float` to read one back.

The rule has a flip side: a free function may not *take* one of
those names. The compiler answers `abs(x)`, `len(s)`, `min(a, b)`,
`print(…)` and the rest at the call site, before it looks at what
your program declares, so a `fn abs(...)` of your own could never
be the one that runs — and until this was refused, it wasn't: the
program built and printed the builtin's answer. Now the
declaration is refused where you wrote it:

```hale,fragment
fn abs(a: Int) -> Int { return 0 - a; }    // error, at `abs`
```

> `` `abs` is a built-in call form and cannot name a fn; rename it ``

Rename it (`abs_of`, `magnitude`) and everything works. A
*method* may still be called any of them — a method is reached
through a receiver, `b.len()`, which no builtin claims, which is
why the standard library's own ring buffer can declare `fn len()`.
The exact set lives in `spec/tokens.md` § *Built-in identifiers*.

The set is short because most builtins can be told apart from your
function by *what you pass them*. The `bounded[T; N]` operations —
`count`, `clear`, `truncate`, `push`, `at`, `set` — are recognized
only when the first argument is a bounded receiver, so those names
stay yours to declare, and a `fn count(xs: bounded[Int; 8])` of
your own answers calls on a `bounded[Int; 8]` in preference to the
intrinsic. See
*[Collections](../everyday/collections.md)*.

Like the unknown-identifier rule, this one wants the whole
program, so it's on for `hale check <directory>`. One file of a
multi-file project checked on its own stays permissive: it may
well be calling something its sibling declares.

Next: [Control flow](./control-flow.md).
