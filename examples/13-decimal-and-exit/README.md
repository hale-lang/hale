# 13-decimal-and-exit

Decimal literals + arithmetic, plus `return n` from main mapping
to the process exit code.

## What it shows

`Decimal` is a distinct type from `Float` — `1.5d` literal,
`Decimal` ascription, propagating through `+`, `-`, `*`, `/`.
The two paths agree on arithmetic for values that are exact
multiples of representable f64 quantities (the trellis-demo
case); for inputs like `100.40d` that aren't representable, v0
output may differ in the last few digits between interpreter
(Rust f64 `Display`, shortest-round-trip) and codegen (`printf
%g`, 6 significant digits).

```
$ lotus run examples/13-decimal-and-exit/main.lt
bid=100.40 ask=100.45
spread=0.04999999999999716
mid=100.42500000000001
fee=0.10042500000000001
$ echo $?
0

$ lotus build examples/13-decimal-and-exit/main.lt
$ ./examples/13-decimal-and-exit/main
bid=100.4 ask=100.45
spread=0.05
mid=100.425
fee=0.100425
$ echo $?
0
```

The numeric values are equivalent up to f64 representation; the
differing display formats are a v0 hack. Real fixed-point or
arbitrary-precision Decimal lands with the trellis production
deployment work, where price-tick precision actually matters.

## Why this is interesting

This is the codegen-arc piece that keeps the path moving toward
`trellis-demo` as a build target. Decimals + return-from-main
are both small individually but bring the codegen surface
closer to what the trellis pipeline writes. Closures and
modes are the two remaining big chunks.
