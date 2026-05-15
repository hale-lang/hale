# Aperio — Claude Code entry point

This repo is the Aperio language: compiler, runtime, stdlib,
spec. Contributor briefs are organized by role under
[`agents/`](./agents/) — read the one that matches what the
user is asking about.

## Routing

| If the user wants to…                              | Read |
|----------------------------------------------------|------|
| Write or debug an Aperio program                   | [`agents/app-dev.md`](./agents/app-dev.md) |
| Add to the stdlib or write an Aperio library      | [`agents/library-dev.md`](./agents/library-dev.md) |
| Work on the compiler / runtime / spec             | [`agents/compiler-dev.md`](./agents/compiler-dev.md) |

The briefs are self-contained. Don't read multiple at once;
pick the role that matches the task.

## Repo conventions (apply to every role)

- **Aperio** is the language. **lotus** is the runtime
  substrate. C-runtime symbols stay `lotus_*` by design.
- The spec under `spec/` is the canonical contract. It
  describes shipped behavior, not aspirations. If the impl
  changes user-visible behavior, the spec changes in the same
  commit.
- File extension `.ap`. ASCII-only outside string literals and
  comments. Statements end with `;`.
- The user owns commit cadence — never commit without an
  explicit ask.
- Don't generate planning / status / progress markdown files
  in the repo. Use conversation, the friction log, or
  `notes/open-questions.md`.

## Build + test

```sh
cargo build --release
cargo test --release --workspace -- --test-threads=1
```

The serial flag avoids "text file busy" flakes from parallel
test binaries racing each other on the same temp path.
