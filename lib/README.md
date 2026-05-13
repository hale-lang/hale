# lib — domain types shared across MOA apps

> Application-domain code that's neither MOA substrate nor stdlib.
> Empty scaffold at v1; populated as sharing between apps emerges.

## What lib/ is for

When two or more MOA apps need the same domain types — `Order`,
`Trade`, `Account` in a trading platform; `Tower`, `Flower`,
`Braid`, `Pulse` shared between an IDE and its observation tooling;
`Document`, `Section`, `Header` shared between a docs platform and
its renderer — those types live here.

This is the *application-domain* layer, distinct from:

- **`moa/`** — the architectural substrate. Types every MOA app uses
  regardless of domain (`LocusId`, `RuntimeEvent`, `Tick`).
- **`crates/aperio-codegen/runtime/stdlib/`** — language stdlib.
  Bundled with the compiler. Things every Aperio program might use
  (`std::io`, `std::log`, `std::http`).
- **`apps/`** — leaf applications. The actual programs that compose
  the substrate + stdlib + lib into running systems.

## What goes here

- A directory per shared domain (`lib/trading/`, `lib/ide-types/`,
  `lib/docs/`).
- Each domain directory holds its own `.ap` files of shared `type`
  declarations and possibly shared `locus` declarations (services
  multiple apps would consume).
- A `README.md` per domain explaining its scope and which apps
  consume it.

## How shared code reaches apps

At v1, Aperio's per-directory seed model (F.19) makes every `.ap`
file in one directory visible to every other in that directory; but
**cross-seed imports are not yet shipped** (F.19's open question
Q18). So at v1, `lib/<domain>/` files cannot yet be `import`-ed by
apps in `apps/<name>/`. The practical paths until cross-seed
imports land:

1. **Wait** — leave `lib/` scaffolded. Add inhabitants when the
   second consumer for a domain emerges; ship them once cross-seed
   imports land.
2. **Compiler-bundle** — wire selected `lib/<domain>/` directories
   into the compiler's path-resolution table (`STDLIB_PATH_RENAMES`
   style), so `lib::trading::Order` resolves the same way
   `moa::RuntimeEvent` does. Heavier; only worth it when a domain
   is genuinely substrate-shaped.
3. **Vendor** — copy lib contents into each consuming app's seed.
   Friction-laden; only justified when (1) and (2) are both blocked.

The plan at v1 is option (1) — scaffold the directory, document the
intent, populate when cross-seed imports unblock it.

## Naming conventions

- Directories: `lib/<domain>/` — lowercase, hyphen-separated if
  multi-word (`lib/ide-types/`, not `lib/IdeTypes/`).
- Files within: snake_case (`order.ap`, `account.ap`).
- Types within: PascalCase, no `L` suffix (since they are types,
  not loci) — `Order`, `Trade`, `Account`.
- If a domain ships any locus, that locus follows the styleguide
  `<Name>L` suffix.

## Cross-references

- `moa/README.md` — the substrate layer this builds on
- `moa/MOA.md` — the architectural pattern that motivates sharing
- `notes/aperio-seed.md` — F.19 seed model and the cross-seed-
  imports open question
- `spec/styleguide.md` — pattern catalog, naming conventions
