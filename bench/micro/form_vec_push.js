// JS equivalent of form_vec_push.ap.
// Native Array.push from empty; V8's underlying storage is
// generally a packed Int (SMI) array for this access pattern.

const iters = 500_000;
const v = [];
const t0 = process.hrtime.bigint();
for (let i = 0; i < iters; i++) {
    v.push(i);
}
const elapsed = process.hrtime.bigint() - t0;
console.log(`iters=${iters}`);
console.log(`len=${v.length}`);
console.log(`elapsed_ns=${elapsed}`);
