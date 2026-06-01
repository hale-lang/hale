# Your first run

> Put something on screen, and see the two run modes.

Create a file `hello.hl`:

```hale
fn main() {
    println("Hello from Hale.");
}
```

Run it through the interpreter:

```sh
hale run hello.hl
```

```
Hello from Hale.
```

Compile it to a native binary and run that:

```sh
hale build hello.hl
./hello
```

Same output. The interpreter is for fast feedback; `build`
produces the artifact you ship.

## What's here

- **`fn main()`** is the entry point, the same as it is in C, Go,
  or Rust. A Hale program starts by calling it.
- **`println(...)`** prints its arguments followed by a newline.
  It takes *any number* of arguments and concatenates them —
  there's no format string:

  ```hale
  fn main() {
      let name = "Hale";
      println("Hello from ", name, ".");
  }
  ```

- **Statements end with `;`.** Newlines are just whitespace —
  they don't end statements. Source is ASCII outside of string
  literals and comments.

Comments are C-style:

```hale
// a line comment
/* a block comment */
```

That's the whole surface you need to start. The next chapter
introduces variables and the value types — the vocabulary every
Hale program is built from.

> **A note on `hale run` vs files with imports.** The
> interpreter doesn't resolve cross-file library imports yet; for
> any program that uses `import "..." as ...;` use `hale build`.
> Single-file programs and whole-directory builds work in both.
