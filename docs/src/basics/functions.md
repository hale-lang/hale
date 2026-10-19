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

A trailing comma after the last argument is fine — `add(2, 3,)` —
which keeps a call whose arguments sit one per line diff-friendly.

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
- **Defaults are evaluated at the call site**, each time a call
  leaves one, not baked in when the function is defined.
- **A default reads only constants and the module's items**, never
  the caller's scope. It may read literals, constants, free functions,
  types and variants, and builtins. It may not read a caller's local, a
  function value bound there, or an earlier parameter: `hale check`
  refuses those where the default is written. A caller binding with
  the name of a constant the default reads (`let K` beside `const K`)
  is refused at the call that leaves the default, since the call would
  read the binding: rename it or pass the value. A
  default that builds a locus builds it in the caller too, so a
  worker on another pool that leaves such an argument out may be
  making a [cross-pool spawn](../services/parents-children.md),
  which `hale check` refuses at that call. A default never
  publishes: a `<-` written in one is refused, since the caller's
  scope holds none of the declaring locus's bus; call a method that
  publishes instead.

Locus methods support defaults too. One caveat: bus-handler
methods and mode methods reject them — their argument shape is
fixed by the runtime, so there's no slot to fill at dispatch time.

## Bounded type parameters

A type parameter may carry one bound, `Numeric`: the argument is an
`Int`, a `Float`, a `Decimal` or a `Duration`.

```hale
fn twice<T: Numeric>(x: T) -> T {
    return x + x;
}
```

`twice(2)` and `twice(1.5)` check; `twice("ab")` is refused at the
call, and `Box<String>` of a `type Box<T: Numeric>` at the type
argument. `Numeric` is the only bound — there is no trait system, so
`<T: Eq>` is refused where it is declared.

## Generic calls inside loci

A generic function can also be called from a generic locus. If a
`Holder<T>` calls `first(self.value)`, a `Holder<Int>` uses the `Int`
call and a `Holder<String>` uses the `String` call. Binding the field
to a local first has the same result. Params defaults use the type
arguments of the locus declaring them.

Function and method defaults read nothing of the caller, so a default
that calls `first(42)` is the same call at every invocation. The
compiler still keeps a checked call row for each invocation, including
defaults that call other defaulted functions.

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

For the same reason a locus's method is not a function value:
`apply_twice(self.inc, 10)` would need the locus along with the
function, so `hale check` refuses it at `inc`. Call the method
where you need its result, or pass a free function.

A function that can fail (`-> Int fallible(E)`) is not a function
value either: whoever calls through the value would have no `or` to
handle the failure with. Pass a small wrapper that handles it
(`fn parse_or(s: String) -> Int { return parse(s) or 0; }`); `hale
check` suggests one.

A function that returns a locus is a value when it builds what it
returns (`fn make() -> Worker { return Worker { }; }`): whoever calls
through the value owns the fresh instance, as a direct call's caller
does. One that may hand back a locus it was given (`fn pick(w:
Worker) -> Worker { return w; }`) is refused at its name; call it
directly instead.
Such a value can come out of a call too (`let f = pick();` with `fn
pick() -> fn() -> Worker`), out of an array of them in a `for` loop, or
out of a tuple; however you got it, each `Worker` a call through it
builds is yours, as a direct call's would be. A call
whose result you drop (`f();`) tears the `Worker` down at the `;`.

A stdlib function written in Hale passes the same way
(`apply(std::url::encode, s)`). One the compiler answers itself at a
call (`std::math::abs`) has no function behind it to pass; `hale
check` says so and suggests the one-line wrapper to pass instead.

A function value in scope shadows a function of the same name, in a
call as anywhere else: `fn apply(f: fn(Int) -> Int, x: Int) -> Int {
return f(x); }` calls the `f` it was handed, even beside a `fn f`. The
same goes for a builtin's name: a parameter called `len` is what
`len(s)` calls inside that function. What counts is that the binding is
declared as a function (a `fn(..)` type, or a function's name as its
value); a plain `let max = 10;` leaves `max(1, 2)` the builtin.

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

A call is held to the same standard as a read: the callee has to
name something. A misspelled call is an error at the call, not a
mystery from the backend later:

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
`abs` / `min` / `max`, `starts_with` / `contains`, the
verification accumulators `sum` / `prod` and `check_closures()`,
and the `bounded` collection intrinsics. Those are not magic names to
memorise — you'll meet each one where it's useful — but they are
why `len(s)` needs no import. They are typed like any other call:
`len(s)` is an `Int`, `to_string(x)` a `String`, `abs(x)` whatever
`x` is, so `let n: String = len(s)` is refused at the `let`.

A builtin call is the same call wherever you write it. `len(s)` in
the middle of an expression and `len(s);` on a line of its own are
one name with one meaning: the second evaluates it and throws the
answer away. Discarding the answer is rarely what you meant, but
it is not a different vocabulary, and the compiler no longer treats
it as one.

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
program, so it's on for `hale check <directory>` and for every
build — `build`, `run` and `test` compile exactly what they bundle,
so the line you read is the same one `check` would have shown you.
One file of a multi-file project checked on its own stays
permissive: it may well be calling something its sibling declares.

Next: [Control flow](./control-flow.md).
