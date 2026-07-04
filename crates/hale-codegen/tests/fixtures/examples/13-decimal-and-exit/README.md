# 13-decimal-and-exit

Decimal literals + arithmetic, plus `return n` from main mapping
to the process exit code.

## What it shows

`Decimal` is a distinct type from `Float` — `1.5d` literal,
`Decimal` ascription, propagating through `+`, `-`, `*`, `/`.
`Decimal` is an exact inline i128 mantissa at scale 9 (mantissa
× 10^-9), so the interpreter and codegen produce identical exact
output — no float noise for inputs like `100.40d`.

```
$ hale run examples/13-decimal-and-exit/main.hl
bid=100.4 ask=100.45
spread=0.05
mid=100.425
fee=0.100425
$ echo $?
0

$ hale build examples/13-decimal-and-exit/main.hl
$ ./examples/13-decimal-and-exit/main
bid=100.4 ask=100.45
spread=0.05
mid=100.425
fee=0.100425
$ echo $?
0
```

The display trims trailing zeros (`100.40d` prints `100.4`), and
both backends agree exactly because the mantissa is stored, not
a floating approximation.

## Why this is interesting

This is the codegen-arc piece that keeps the path moving toward
`fitter-applier-demo` as a build target. Decimals + return-from-main
are both small individually but bring the codegen surface
closer to what the fitter/applier pipeline writes. Closures and
modes are the two remaining big chunks.
