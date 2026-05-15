// JS equivalent of fn_call.ap.
// Free function called in a tight loop. V8 may inline; that's
// the realistic comparison.

function noop(x) {
    return x;
}

const iters = 10_000_000;
const t0 = process.hrtime.bigint();
let acc = 0;
for (let i = 0; i < iters; i++) {
    acc = noop(i);
}
const elapsed = process.hrtime.bigint() - t0;
console.log(`iters=${iters}`);
console.log(`acc=${acc}`);
console.log(`elapsed_ns=${elapsed}`);
