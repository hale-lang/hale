// JS equivalent of loop_overhead.ap.
// XOR accumulation; `| 0` forces 32-bit int ops in V8 so the
// loop doesn't accidentally promote to f64 (which would change
// what we're measuring).

const iters = 100_000_000 + process.pid;
const t0 = process.hrtime.bigint();
let acc = process.pid;
for (let i = 0; i < iters; i++) {
    acc = (acc ^ i) | 0;
}
const elapsed = process.hrtime.bigint() - t0;
console.log(`iters=${iters}`);
console.log(`acc=${acc}`);
console.log(`elapsed_ns=${elapsed}`);
