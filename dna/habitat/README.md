# The habitat

The habitat is the organism the toolchain ships. An organism exists to
keep a heart beating; the habitat exists to keep organisms placed. Its
heart slot is empty: what it keeps alive is other organisms' bodies, and
its substrate is its own body and the backends its genome declares —
memory (Postgres), the nerves (NATS) and the senses' store — which it
holds as allocations, one set per organism, named for the organism's
record identity and the role.

`hale habitat up` births it the first time, without a parent (it is the
one organism that may be), and starts its body; `hale habitat status`
reads it; `hale habitat down` stops its body and its services and keeps
their data.

- `axiom` **The habitat may be born without a parent.** Every other
  organism is grafted from it, as its child.
- `law` **Only the habitat's acts provision.** Birth, `hale habitat
  migrate` and the habitat's upgrade bring an organism's allocations up
  and move their schemas; no command an organism runs provisions or
  migrates anything as a side effect.
- `law` **An allocation records the versions it was migrated to.** Each
  allocation role's schema version is recorded beside the allocation, and
  an allocation behind the toolchain's version is migrated by the
  habitat's upgrade, never in the middle of an organism's work.
- `law` **The habitat's data outlives its body.** Stopping the habitat
  stops its body and its services; only a test's habitat, under a scratch
  root, is ever taken down with its data.
