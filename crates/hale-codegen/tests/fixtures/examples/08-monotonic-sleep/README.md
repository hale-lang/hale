# 08 — monotonic sleep

Exercises `time::sleep` on the codegen path. Each call lowers to
`clock_nanosleep(CLOCK_MONOTONIC, 0, &req, &rem)` with EINTR retry,
matching the interpreter's primitive (also `clock_nanosleep` on the
monotonic clock, via `libc`).

```
$ hale run   examples/08-monotonic-sleep/main.hl
tick 0
tick 1
tick 2
done

$ hale build examples/08-monotonic-sleep/main.hl
built: examples/08-monotonic-sleep/main
$ time ./examples/08-monotonic-sleep/main
tick 0
tick 1
tick 2
done

real	0m0.150s
```

## Clock discipline

Hale grounds every scheduling decision on `CLOCK_MONOTONIC`. NTP
slewing and wall-clock jumps cannot warp a `time::sleep` interval;
EINTR delivers a `rem` that the retry loop resumes from, so a
delivered signal does not shorten the total sleep.

`CLOCK_REALTIME` is reserved for `time::now()` (wall-clock
observation only) and never used for scheduling. This applies to
both the interpreter and the codegen path; both compile down to
the same primitive.

## What this example does NOT yet exercise

- `run()` lifecycle method (waits on locus-as-struct runtime ABI)
- `time::now()` / `time::monotonic()` (next time-module commit)
- Cooperative scheduler (Phase 2)
