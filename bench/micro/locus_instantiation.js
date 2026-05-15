// JS equivalent of locus_instantiation.ap.
// Matches the helper-fn + field-read shape.

class Empty {
    constructor(v) { this.v = v; }
    read() { return this.v; }
}

function instantiateOne(seed) {
    const e = new Empty(seed);
    return e.read();
}

const iters = 100_000;
const pid = process.pid;
const t0 = process.hrtime.bigint();
let sink = 0;
for (let i = 0; i < iters; i++) {
    sink = (sink ^ instantiateOne(i + pid)) | 0;
}
const elapsed = process.hrtime.bigint() - t0;
console.log(`iters=${iters}`);
console.log(`sink=${sink}`);
console.log(`elapsed_ns=${elapsed}`);
